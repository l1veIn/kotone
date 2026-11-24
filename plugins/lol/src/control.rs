//! 操作客户端（ADR 035 D1 2026-11-24 玩家改了、ADR 038 第 3.5 步）：自动接受对局、自动选 / 禁英雄，以及给她的
//! `lol.client`。照 LeagueAkari（MIT，`dev` 分支 `src/main/shards/auto-gameflow`、`auto-select`）：
//!
//! - **接受对局**：阶段是 `ReadyCheck`、自己还没回应（`playerResponse` 是 `None`）时，等玩家设的几秒，
//!   `POST /lol-matchmaking/v1/ready-check/accept`。玩家自己点了接受 / 拒绝就不管；
//! - **选 / 禁英雄**：`GET /lol-champ-select/v1/session`，当前这一轮（第一组还有没做完的动作）里轮到自己
//!   （`actorCellId` = `localPlayerCellId`）、正在进行（`isInProgress`）的那个动作；按玩家排好的顺序挑第一个
//!   能选 / 能禁的（`pickable-champion-ids` / `bannable-champion-ids`，没被禁、没被别人选；禁的时候不禁队友想玩的），
//!   等几秒，`PATCH /lol-champ-select/v1/session/actions/{id}` `{championId, completed}`；`completed` 是“锁定”，
//!   不锁定就只是亮出来（玩家自己再点锁定）。每个动作只做一次。
//!
//! 读和写都只走客户端自己的本机接口（LCU），不读游戏内存、不注入。数据开关 `control` **默认关**，风险提示写着
//! LeagueAkari #523（外服有人用了自动接受、自动选英雄后被永久封号）。

use serde::Deserialize;
use serde_json::{json, Value};

/// “操作客户端”开关（plugin.toml 的 sources）。
pub const SOURCE_CONTROL: &str = "control";

pub mod paths {
    pub const READY_CHECK: &str = "/lol-matchmaking/v1/ready-check";
    pub const ACCEPT: &str = "/lol-matchmaking/v1/ready-check/accept";
    pub const DECLINE: &str = "/lol-matchmaking/v1/ready-check/decline";
    pub const CHAMP_SELECT: &str = "/lol-champ-select/v1/session";
    pub const PICKABLE: &str = "/lol-champ-select/v1/pickable-champion-ids";
    pub const BANNABLE: &str = "/lol-champ-select/v1/bannable-champion-ids";

    pub fn action(id: i64) -> String {
        format!("/lol-champ-select/v1/session/actions/{id}")
    }
}

/// 玩家的偏好（设置 `plugins.lol.settings`，camelCase）。
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ControlSettings {
    /// 自动接受对局。
    pub auto_accept: bool,
    /// 接受前等几秒（0–8，照 Akari：给玩家反悔的时间；确认对局一共约 10 秒）。
    pub accept_delay_seconds: u32,
    pub pick: SelectConfig,
    pub ban: SelectConfig,
}

/// 自动选 / 禁的一份偏好。
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SelectConfig {
    pub enabled: bool,
    /// 按顺序，第一个能用的。
    pub champions: Vec<i64>,
    /// 选了就锁定（false = 只亮出来，玩家自己锁）。禁英雄总是直接禁。
    pub lock_in: bool,
    /// 轮到自己以后等几秒（0–20）。
    pub delay_seconds: u32,
}

impl ControlSettings {
    pub fn from_plugin_settings(settings: &Value) -> Self {
        let mut s: Self = serde_json::from_value(settings.clone()).unwrap_or_default();
        s.accept_delay_seconds = s.accept_delay_seconds.min(8);
        s.pick.delay_seconds = s.pick.delay_seconds.min(20);
        s.ban.delay_seconds = s.ban.delay_seconds.min(20);
        s
    }
}

/// 要选还是禁。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Pick,
    Ban,
}

/// 轮到自己的那个动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MyAction {
    pub id: i64,
    pub kind: Kind,
}

fn i(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(Value::as_i64)
}

/// 现在轮到自己的动作（照 Akari `getCurrentActions` + `getActiveAction`）：第一组还有没做完的动作里，
/// 自己的、没做完的、正在进行的那一个。
pub fn my_action(session: &Value) -> Option<MyAction> {
    let me = i(session, "localPlayerCellId")?;
    let groups = session.get("actions")?.as_array()?;
    let current = groups.iter().find(|g| {
        g.as_array().is_some_and(|a| {
            a.iter()
                .any(|x| x.get("completed").and_then(Value::as_bool) != Some(true))
        })
    })?;
    let action = current.as_array()?.iter().find(|a| {
        i(a, "actorCellId") == Some(me)
            && a.get("completed").and_then(Value::as_bool) != Some(true)
            && a.get("isInProgress").and_then(Value::as_bool) == Some(true)
    })?;
    let kind = match action.get("type").and_then(Value::as_str)? {
        "pick" => Kind::Pick,
        "ban" => Kind::Ban,
        _ => return None,
    };
    Some(MyAction {
        id: i(action, "id")?,
        kind,
    })
}

/// 已经被禁的、被别人选了的英雄（不能再选 / 禁）。
fn taken(session: &Value, me: Option<i64>) -> Vec<i64> {
    let mut out = Vec::new();
    for key in ["myTeamBans", "theirTeamBans"] {
        if let Some(list) = session
            .pointer(&format!("/bans/{key}"))
            .and_then(Value::as_array)
        {
            out.extend(list.iter().filter_map(Value::as_i64));
        }
    }
    for group in session
        .get("actions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for a in group.as_array().into_iter().flatten() {
            let done = a.get("completed").and_then(Value::as_bool) == Some(true);
            let champion = i(a, "championId").unwrap_or(0);
            // 做完的禁用、别人做完的选择
            if done
                && champion > 0
                && (a.get("type").and_then(Value::as_str) == Some("ban")
                    || i(a, "actorCellId") != me)
            {
                out.push(champion);
            }
        }
    }
    for team in ["myTeam", "theirTeam"] {
        for m in session
            .get(team)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if i(m, "cellId") != me {
                if let Some(c) = i(m, "championId").filter(|c| *c > 0) {
                    out.push(c);
                }
            }
        }
    }
    out
}

/// 队友想玩的（`championPickIntent`）：禁英雄时不禁它们（照 Akari）。
fn teammates_intents(session: &Value, me: Option<i64>) -> Vec<i64> {
    session
        .get("myTeam")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|m| i(m, "cellId") != me)
        .flat_map(|m| [i(m, "championPickIntent"), i(m, "championId")])
        .flatten()
        .filter(|c| *c > 0)
        .collect()
}

/// 这一次要发的：`(动作 id, 英雄, 锁定)`。`available`：客户端说能选 / 能禁的英雄（`pickable` / `bannable`）。
pub fn decide(
    session: &Value,
    action: &MyAction,
    config: &SelectConfig,
    available: &[i64],
) -> Option<(i64, i64, bool)> {
    if !config.enabled || config.champions.is_empty() {
        return None;
    }
    let me = i(session, "localPlayerCellId");
    let taken = taken(session, me);
    let intents = if action.kind == Kind::Ban {
        teammates_intents(session, me)
    } else {
        Vec::new()
    };
    let champion = config.champions.iter().copied().find(|c| {
        (available.is_empty() || available.contains(c))
            && !taken.contains(c)
            && !intents.contains(c)
    })?;
    let complete = action.kind == Kind::Ban || config.lock_in;
    Some((action.id, champion, complete))
}

/// `PATCH /actions/{id}` 的请求体。
pub fn action_body(champion: i64, complete: bool) -> Value {
    json!({"championId": champion, "completed": complete})
}

/// 准备确认：还要不要自动接受（还在等、自己没回应过）。
pub fn should_accept(ready_check: &Value) -> bool {
    ready_check.get("state").and_then(Value::as_str) == Some("InProgress")
        && ready_check.get("playerResponse").and_then(Value::as_str) == Some("None")
}

// ---------- 每一轮和工具 ----------

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use kotone_core::log;
use kotone_plugin_api::{Host, ToolResult};

use crate::client::Lcu;
use crate::LolPlugin;

/// 操作客户端这一路的状态（每个准备确认、每个选 / 禁动作只做一次）。
#[derive(Default)]
pub(crate) struct ControlState {
    /// 这次准备确认是什么时候开始的（等玩家设的几秒）。
    ready_since: Option<Instant>,
    /// 轮到自己的动作是什么时候开始的。
    action_since: HashMap<i64, Instant>,
    /// 已经做过的动作。
    done: HashSet<i64>,
}

fn ids(v: &Value) -> Vec<i64> {
    v.as_array()
        .map(|a| a.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default()
}

fn status(status: &str, text: impl Into<String>) -> Value {
    json!({"status": status, "text": text.into()})
}

impl LolPlugin {
    /// 每一轮（客户端开着时）：开关和偏好都开着才动。
    pub(crate) async fn run_control(&self, host: &dyn Host, lcu: &Lcu, phase: &str) {
        if !host.source_enabled(SOURCE_CONTROL) {
            *self.control.lock().unwrap() = ControlState::default();
            return;
        }
        let prefs = ControlSettings::from_plugin_settings(&host.settings());
        match phase {
            "ReadyCheck" if prefs.auto_accept => {
                let Ok(rc) = lcu.get(paths::READY_CHECK).await else {
                    return;
                };
                if !should_accept(&rc) {
                    self.control.lock().unwrap().ready_since = None;
                    return;
                }
                let since = *self
                    .control
                    .lock()
                    .unwrap()
                    .ready_since
                    .get_or_insert_with(Instant::now);
                if since.elapsed() >= Duration::from_secs(u64::from(prefs.accept_delay_seconds)) {
                    let result = lcu.post(paths::ACCEPT, &Value::Null).await;
                    self.control.lock().unwrap().ready_since = None;
                    log::log(&match result {
                        Ok(_) => "lol: 自动接受了对局".to_string(),
                        Err(e) => format!("lol: 自动接受对局失败：{e}"),
                    });
                }
            }
            "ChampSelect" if prefs.pick.enabled || prefs.ban.enabled => {
                let Ok(session) = lcu.get(paths::CHAMP_SELECT).await else {
                    return;
                };
                let Some(action) = my_action(&session) else {
                    return;
                };
                let config = match action.kind {
                    Kind::Pick => &prefs.pick,
                    Kind::Ban => &prefs.ban,
                };
                {
                    let mut c = self.control.lock().unwrap();
                    if !config.enabled || c.done.contains(&action.id) {
                        return;
                    }
                    let since = *c.action_since.entry(action.id).or_insert_with(Instant::now);
                    if since.elapsed() < Duration::from_secs(u64::from(config.delay_seconds)) {
                        return;
                    }
                }
                let list = match action.kind {
                    Kind::Pick => paths::PICKABLE,
                    Kind::Ban => paths::BANNABLE,
                };
                let available = lcu.get(list).await.map(|v| ids(&v)).unwrap_or_default();
                let Some((id, champion, complete)) = decide(&session, &action, config, &available)
                else {
                    return;
                };
                let result = lcu
                    .patch(&paths::action(id), &action_body(champion, complete))
                    .await;
                // 成不成功都只试一次（失败了反复发会刷屏，玩家自己点）
                self.control.lock().unwrap().done.insert(id);
                let what = match action.kind {
                    Kind::Pick if complete => "选并锁定",
                    Kind::Pick => "亮出",
                    Kind::Ban => "禁",
                };
                log::log(&match result {
                    Ok(_) => format!("lol: 自动{what}了英雄 {champion}"),
                    Err(e) => format!("lol: 自动{what}英雄 {champion} 失败：{e}"),
                });
            }
            "ChampSelect" | "ReadyCheck" => {}
            _ => *self.control.lock().unwrap() = ControlState::default(),
        }
    }

    /// `lol.client`：玩家说了，她替玩家接受 / 拒绝对局、选 / 禁 / 亮出某个英雄。
    pub(crate) async fn client_tool(
        &self,
        host: &dyn Host,
        lcu: &Lcu,
        arguments: &Value,
    ) -> ToolResult {
        if !host.source_enabled(SOURCE_CONTROL) {
            return ToolResult::content(status(
                "disabled",
                "英雄联盟插件的“操作客户端”关着（插件 → 英雄联盟 → 设置 里可以打开，有封号风险）",
            ));
        }
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("");
        match action {
            "accept" | "decline" => {
                let rc = lcu.get(paths::READY_CHECK).await.unwrap_or(Value::Null);
                if rc.get("state").and_then(Value::as_str) != Some("InProgress") {
                    return ToolResult::content(status("not_now", "现在没有要确认的对局"));
                }
                let path = if action == "accept" {
                    paths::ACCEPT
                } else {
                    paths::DECLINE
                };
                ToolResult::content(match lcu.post(path, &Value::Null).await {
                    Ok(_) if action == "accept" => status("ok", "接受了对局"),
                    Ok(_) => status("ok", "拒绝了对局"),
                    Err(e) => status("error", format!("没成功：{e}")),
                })
            }
            "pick" | "ban" | "hover" => {
                let Some(query) = arguments
                    .get("champion")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|q| !q.is_empty())
                else {
                    return ToolResult::content(status("error", "要说是哪个英雄（champion）"));
                };
                let champion = match self.champion_id_for_client(host, query).await {
                    Some(id) => id,
                    None => {
                        return ToolResult::content(status(
                            "not_found",
                            format!("不知道“{query}”是哪个英雄"),
                        ))
                    }
                };
                let Ok(session) = lcu.get(paths::CHAMP_SELECT).await else {
                    return ToolResult::content(status("not_now", "现在不在选英雄"));
                };
                let want = if action == "ban" {
                    Kind::Ban
                } else {
                    Kind::Pick
                };
                let current = my_action(&session).filter(|a| a.kind == want);
                // 亮出（hover）可以在没轮到时对自己的选英雄动作做（照 Akari 的 intent）
                let target = match (current, action) {
                    (Some(a), _) => Some(a.id),
                    (None, "hover") => first_pick_action(&session),
                    _ => None,
                };
                let Some(id) = target else {
                    return ToolResult::content(status(
                        "not_now",
                        if want == Kind::Ban {
                            "现在没轮到你禁英雄"
                        } else {
                            "现在没轮到你选英雄"
                        },
                    ));
                };
                let body = if action == "hover" {
                    json!({"championId": champion})
                } else {
                    action_body(champion, true)
                };
                let result = lcu.patch(&paths::action(id), &body).await;
                self.control.lock().unwrap().done.insert(id);
                ToolResult::content(match result {
                    Ok(_) => status(
                        "ok",
                        match action {
                            "ban" => "禁了",
                            "hover" => "亮出来了（还没锁定）",
                            _ => "选好并锁定了",
                        },
                    ),
                    Err(e) => status("error", format!("客户端没接受：{e}")),
                })
            }
            _ => ToolResult::content(status(
                "error",
                "action 只能是 accept / decline / pick / ban / hover",
            )),
        }
    }

    /// 英雄名 → id：先用客户端的资料（这时客户端一定开着），再用网上的外号表。
    async fn champion_id_for_client(&self, host: &dyn Host, query: &str) -> Option<i64> {
        if let Some(data) = self.loaded_data() {
            let hits: Vec<i64> = data
                .champions
                .iter()
                .filter(|c| c.matches(query))
                .map(|c| c.id)
                .collect();
            if hits.len() == 1 {
                return hits.first().copied();
            }
        }
        if host.source_enabled(crate::SOURCE_STATS) {
            if let Ok(names) = self.stats_for(host).nicknames().await {
                return names.find(query);
            }
        }
        None
    }

    /// `lol.champions`（只给页面）：全部英雄的名字和 id（设置里排自动选 / 禁的顺序用）。
    pub(crate) async fn champions_tool(&self, host: &dyn Host) -> ToolResult {
        let mut list: Vec<Value> = match self.loaded_data() {
            Some(data) if !data.champions.is_empty() => data
                .champions
                .iter()
                .map(|c| json!({"id": c.id, "label": c.label(), "short": c.short()}))
                .collect(),
            _ => match self.stats_for(host).nicknames().await {
                Ok(n) => n
                    .names
                    .keys()
                    .map(|id| json!({"id": id, "label": n.label(*id), "short": n.short(*id)}))
                    .collect(),
                Err(e) => {
                    return ToolResult::content(status("error", format!("英雄列表拿不到：{e}")))
                }
            },
        };
        list.sort_by(|a, b| a["short"].as_str().cmp(&b["short"].as_str()));
        ToolResult::content(json!({"status": "ok", "count": list.len()}))
            .with_page(crate::pages::PAGE_SETTINGS, json!({"champions": list}))
    }
}

/// 自己的第一个还没做的选英雄动作（亮出用，照 Akari `getFirstUnfinishedPickAction`）。
pub fn first_pick_action(session: &Value) -> Option<i64> {
    let me = session.get("localPlayerCellId").and_then(Value::as_i64)?;
    session
        .get("actions")?
        .as_array()?
        .iter()
        .filter_map(Value::as_array)
        .flatten()
        .find(|a| {
            a.get("actorCellId").and_then(Value::as_i64) == Some(me)
                && a.get("type").and_then(Value::as_str) == Some("pick")
                && a.get("completed").and_then(Value::as_bool) != Some(true)
        })
        .and_then(|a| a.get("id").and_then(Value::as_i64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(actions: Value) -> Value {
        json!({
            "localPlayerCellId": 2,
            "actions": actions,
            "bans": {"myTeamBans": [157], "theirTeamBans": []},
            "myTeam": [
                {"cellId": 1, "championId": 0, "championPickIntent": 39},
                {"cellId": 2, "championId": 0, "championPickIntent": 0},
            ],
            "theirTeam": [{"cellId": 6, "championId": 103}],
        })
    }

    #[test]
    fn my_ban_turn_skips_banned_and_teammates_wishes() {
        let s = session(json!([
            [{"id": 10, "actorCellId": 1, "type": "ban", "completed": false, "isInProgress": true},
             {"id": 11, "actorCellId": 2, "type": "ban", "completed": false, "isInProgress": true}],
            [{"id": 20, "actorCellId": 2, "type": "pick", "completed": false, "isInProgress": false}]
        ]));
        let action = my_action(&s).unwrap();
        assert_eq!(
            action,
            MyAction {
                id: 11,
                kind: Kind::Ban
            }
        );
        let ban = SelectConfig {
            enabled: true,
            champions: vec![157, 39, 64, 86],
            lock_in: false,
            delay_seconds: 0,
        };
        // 157 已经被禁，39 是队友想玩的 → 64
        assert_eq!(
            decide(&s, &action, &ban, &[]),
            Some((11, 64, true)),
            "禁英雄总是直接禁"
        );
        // 客户端说 64 不能禁 → 86
        assert_eq!(decide(&s, &action, &ban, &[86]), Some((11, 86, true)));
        let off = SelectConfig {
            enabled: false,
            ..ban.clone()
        };
        assert_eq!(decide(&s, &action, &off, &[]), None);
    }

    #[test]
    fn my_pick_turn_respects_lock_in_and_taken_champions() {
        let s = session(json!([
            [{"id": 10, "actorCellId": 1, "type": "ban", "completed": true, "championId": 64}],
            [{"id": 20, "actorCellId": 6, "type": "pick", "completed": true, "championId": 103},
             {"id": 21, "actorCellId": 2, "type": "pick", "completed": false, "isInProgress": true}]
        ]));
        let action = my_action(&s).unwrap();
        assert_eq!(action.kind, Kind::Pick);
        let pick = SelectConfig {
            enabled: true,
            champions: vec![64, 103, 39],
            lock_in: false,
            delay_seconds: 0,
        };
        // 64 被禁、103 被对面选了 → 39（队友想玩，但选的时候不管）；不锁定
        assert_eq!(
            decide(&s, &action, &pick, &[39, 64, 103]),
            Some((21, 39, false))
        );
        let lock = SelectConfig {
            lock_in: true,
            ..pick
        };
        assert_eq!(decide(&s, &action, &lock, &[]).map(|d| d.2), Some(true));
    }

    #[test]
    fn not_my_turn_or_not_started() {
        let s = session(json!([
            [{"id": 10, "actorCellId": 1, "type": "ban", "completed": false, "isInProgress": true},
             {"id": 11, "actorCellId": 2, "type": "ban", "completed": false, "isInProgress": false}]
        ]));
        assert_eq!(my_action(&s), None);
        assert_eq!(
            my_action(&json!({"localPlayerCellId": 2, "actions": []})),
            None
        );
        assert_eq!(my_action(&json!({})), None);
    }

    #[test]
    fn ready_check_and_settings() {
        assert!(should_accept(
            &json!({"state": "InProgress", "playerResponse": "None"})
        ));
        assert!(!should_accept(
            &json!({"state": "InProgress", "playerResponse": "Accepted"})
        ));
        assert!(!should_accept(
            &json!({"state": "InProgress", "playerResponse": "Declined"})
        ));
        let s = ControlSettings::from_plugin_settings(&json!({
            "autoAccept": true, "acceptDelaySeconds": 99,
            "pick": {"enabled": true, "champions": [39], "lockIn": true, "delaySeconds": 3},
            "unknown": 1
        }));
        assert!(s.auto_accept && s.accept_delay_seconds == 8);
        assert_eq!(s.pick.champions, vec![39]);
        assert!(!s.ban.enabled);
        assert_eq!(
            ControlSettings::from_plugin_settings(&json!(null)),
            ControlSettings::default()
        );
        assert_eq!(
            action_body(39, true),
            json!({"championId": 39, "completed": true})
        );
        assert_eq!(paths::action(21), "/lol-champ-select/v1/session/actions/21");
    }
}
