//! 发到游戏（ADR 041 D3，第 2 期）：英雄联盟客户端在前台时（选英雄、房间里），宿主把要发的话交给这里，经客户端
//! 自己的本机聊天接口发出去，**不按键**。对局里不归这里：游戏窗口在前台时宿主按游戏配置按键（D3 的修改）。
//!
//! 数据开关“在客户端里发聊天”（`chat`，默认开，要“读客户端数据”也开着）：它是写操作，不放进写着“只读”的 `client`；
//! 也不归“操作客户端”（`control`，管自动接受、自动选英雄这类替玩家做决定的事）。只在有人要发时发。
//!
//! 照 LeagueAkari 的做法（**只学做法、不抄代码**，ADR 039 D5：`dev` 分支 2026-11-25 是 MIT，v1.3.x–v1.4.3 曾是
//! GPL-3.0，按严的一边）。看的是提交 `4f8981f8` 的：
//! - `src/main/shards/in-game-send/send-executor.ts`：按阶段选路；选英雄、房间里几行用换行连成**一条**，调一次
//!   `POST /lol-chat/v1/conversations/{id}/messages`；
//! - `src/main/shards/league-client/lc-state/index.ts`：会话按 `GET /lol-chat/v1/conversations` 的 `type` 认，
//!   选英雄是 `championSelect` 而且 id 里有 `lol-champ-select`，房间是 `customGame`；
//! - `src/main/shards/ongoing-game/computed-state.ts`：房间（排队、确认时房间还在）、选英雄、对局三种阶段。
//!
//! 规矩：截止时间前一定返回；**只发一次、不重试**（ADR 032、原则 4：客户端没回话时话可能已经发出去了）；
//! 发之前来不及就不发；日志不记文字。

use std::time::{Duration, Instant};

use kotone_core::log;
use kotone_plugin_api::{ChatSend, ChatSent, ChatStop, Host, StopCode};
use serde_json::{json, Value};

use crate::client::http::paths as lcu_paths;
use crate::client::{Lcu, LolError};
use crate::LolPlugin;

/// “在客户端里发聊天”开关（plugin.toml 的 sources）。
pub const SOURCE_CHAT: &str = "chat";

/// 读一次（阶段、会话、房间）最多等多久（再短也不超过离截止还剩的时间）。
const READ_TIMEOUT: Duration = Duration::from_secs(2);
/// 离截止不到这么多就不读、不发（照第 1 期“开始之前估计来不及就不开始”）。
const MIN_LEFT: Duration = Duration::from_millis(300);

pub mod paths {
    pub const CONVERSATIONS: &str = "/lol-chat/v1/conversations";
    pub const LOBBY: &str = "/lol-lobby/v2/lobby";

    pub fn messages(conversation: &str) -> String {
        format!("/lol-chat/v1/conversations/{conversation}/messages")
    }
}

/// 客户端里能发聊天的地方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    ChampSelect,
    Lobby,
}

impl Place {
    /// 给人看的（结果里的 `channelName`、回执）。
    pub fn name(self) -> &'static str {
        match self {
            Place::ChampSelect => "选英雄的聊天",
            Place::Lobby => "房间的聊天",
        }
    }

    /// 会话列表里的 `type`。
    fn conversation_type(self) -> &'static str {
        match self {
            Place::ChampSelect => "championSelect",
            Place::Lobby => "customGame",
        }
    }

    /// 只有自己这边的人看得到时，说是谁。
    fn own_side(self) -> &'static str {
        match self {
            Place::ChampSelect => "只有队友看得到",
            Place::Lobby => "只有房间里的人看得到",
        }
    }
}

fn no_chat(message: impl Into<String>) -> ChatStop {
    ChatStop::new(StopCode::NoChatHere, message)
}

/// 对局阶段（`/lol-gameflow/v1/gameflow-phase`）→ 往哪发；不发时是原因（说怎么了、下一步）。
pub fn place(phase: &str) -> Result<Place, ChatStop> {
    match phase {
        "ChampSelect" => Ok(Place::ChampSelect),
        // 排队、确认时房间还在（照 LeagueAkari：有房间就算房间）
        "Lobby" | "Matchmaking" | "ReadyCheck" => Ok(Place::Lobby),
        "GameStart" | "InProgress" | "Reconnect" => Err(no_chat(
            "现在在对局里：切到游戏窗口再说，客户端里没有对局的聊天，这次没发",
        )),
        // ADR 041 第 2 期只做选人 / 房间；结算的聊天两边都看得到
        "WaitingForStats" | "PreEndOfGame" | "EndOfGame" => {
            Err(no_chat("结算界面的聊天还不支持（对面也看得到），这次没发"))
        }
        _ => Err(no_chat(
            "客户端里现在没有能发的聊天：进房间或选英雄以后再发",
        )),
    }
}

/// 这里的聊天谁看得到：`team`（自己这边）/ `all`（两边）。自定义对局两边的人在同一个房间里；自定义对局的选英雄
/// 有没有分开没核对过，按保守的算成两边都看得到。
pub fn audience(place: Place, data: &Value) -> &'static str {
    let custom = match place {
        Place::ChampSelect => data.get("isCustomGame").and_then(Value::as_bool),
        Place::Lobby => data
            .pointer("/gameConfig/isCustom")
            .and_then(Value::as_bool),
    };
    if custom == Some(true) {
        "all"
    } else {
        "team"
    }
}

/// 要的频道和这里一样才发（ADR 041 第 1 期“发给更多人不退”：这里只有一个能发的地方，没有“退到默认频道”这一说）。
pub fn check_channel(place: Place, wanted: &str, here: &str) -> Result<(), ChatStop> {
    if wanted == here {
        return Ok(());
    }
    let message = match (wanted, here) {
        ("all", _) => format!(
            "{}{}，没有发给所有人的，这次没发",
            place.name(),
            place.own_side()
        ),
        ("team", "all") => format!(
            "自定义对局里{}两边都看得到，这次没发：要发给所有人，就说发到所有人",
            place.name()
        ),
        _ => format!("{}里没有「{wanted}」频道，这次没发", place.name()),
    };
    Err(ChatStop::new(StopCode::NoSuchChannel, message))
}

/// 会话列表里这里的那一个。选英雄只认 id 里有 `lol-champ-select` 的（照 LeagueAkari；为什么没查到，真客户端核对）。
/// id 要拼进路径：有 `/`、`?`、`#`、空白的不要（`@`、`~` 照常）。
pub fn conversation(list: &Value, place: Place) -> Option<String> {
    list.as_array()?
        .iter()
        .filter(|c| c.get("type").and_then(Value::as_str) == Some(place.conversation_type()))
        .filter_map(|c| c.get("id").and_then(Value::as_str))
        .find(|id| {
            (place != Place::ChampSelect || id.contains("lol-champ-select"))
                && !id.is_empty()
                && !id.contains(['/', '?', '#'])
                && !id.contains(char::is_whitespace)
        })
        .map(str::to_string)
}

/// 请求体：几行用换行连成一条（照 LeagueAkari），`type: chat`。只放这两项（LeagueAkari 多填的 `fromId` 这些，
/// 它自己的注释说加不加都行）。
pub fn message_body(lines: &[String]) -> Value {
    json!({"body": lines.join("\n"), "type": "chat"})
}

/// 离截止还剩多少；不够 [`MIN_LEFT`] 就不开始。
fn left(deadline: Instant) -> Result<Duration, ChatStop> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left < MIN_LEFT {
        return Err(ChatStop::new(
            StopCode::TimeUp,
            "时间不够了（调用的期限快到了），没开始发",
        ));
    }
    Ok(left)
}

/// 读的时候出错（还没发）。
fn read_error(error: LolError) -> ChatStop {
    match error {
        LolError::Unavailable => ChatStop::new(
            StopCode::ClientUnavailable,
            "连不上英雄联盟客户端，这次没发：客户端刚开的话等几秒再发",
        ),
        LolError::Timeout => ChatStop::new(
            StopCode::ClientUnavailable,
            "英雄联盟客户端没回话，这次没发：客户端刚开的话等几秒再发",
        ),
        other => ChatStop::new(
            StopCode::ClientUnavailable,
            format!("读英雄联盟客户端出错（{other}），这次没发"),
        ),
    }
}

/// 发的时候出错。连不上、客户端回了错误 = 没发；期限到了、连接中途断了 = 可能已经发出去了（不重发）。
fn post_error(place: Place, error: LolError) -> ChatStop {
    match error {
        LolError::Unavailable => read_error(LolError::Unavailable),
        LolError::Status(404, _) => no_chat(format!(
            "{}已经没了（可能刚进加载），这次没发",
            place.name()
        )),
        LolError::Status(code, message) if message.trim().is_empty() => ChatStop::new(
            StopCode::ClientRejected,
            format!("客户端没收下（返回 {code}），这次没发"),
        ),
        LolError::Status(code, message) => ChatStop::new(
            StopCode::ClientRejected,
            format!("客户端没收下（返回 {code}：{message}），这次没发"),
        ),
        LolError::Timeout | LolError::Other(_) => ChatStop::new(
            StopCode::ClientNoReply,
            "客户端没在期限内回话：这句可能已经发出去了，先看一眼聊天，别再发一遍",
        ),
    }
}

/// 经客户端的聊天接口发（宿主已经整理好行、算好频道）：读阶段 → 看谁看得到 → 找会话 → 发一次。
pub async fn send(lcu: &Lcu, request: &ChatSend) -> Result<ChatSent, ChatStop> {
    let read = |deadline: Instant| left(deadline).map(|l| l.min(READ_TIMEOUT));
    let phase = lcu
        .get_until(lcu_paths::PHASE, read(request.deadline)?)
        .await
        .map_err(read_error)?;
    let place = place(phase.as_str().unwrap_or(""))?;
    let data_path = match place {
        Place::ChampSelect => lcu_paths::CHAMP_SELECT,
        Place::Lobby => paths::LOBBY,
    };
    let data = lcu
        .get_until(data_path, read(request.deadline)?)
        .await
        .map_err(read_error)?;
    let here = audience(place, &data);
    check_channel(place, &request.channel, here)?;
    let list = lcu
        .get_until(paths::CONVERSATIONS, read(request.deadline)?)
        .await
        .map_err(read_error)?;
    let Some(id) = conversation(&list, place) else {
        return Err(no_chat(format!(
            "{}还没连上，这次没发：过一两秒再发",
            place.name()
        )));
    };
    if request.cancel.is_cancelled() {
        return Err(ChatStop::new(StopCode::Cancelled, "发送已取消"));
    }
    let timeout = left(request.deadline)?;
    lcu.post_until(
        &paths::messages(&id),
        &message_body(&request.lines),
        timeout,
    )
    .await
    .map_err(|e| post_error(place, e))?;
    Ok(ChatSent {
        channel: here.to_string(),
        channel_name: place.name().to_string(),
        sent_lines: request.lines.len(),
        stopped: None,
    })
}

impl LolPlugin {
    /// `Plugin::send_to_game`：开关再看一遍（宿主看过了，防中途关掉），拿客户端的连接，发。
    pub(crate) async fn send_chat(
        &self,
        host: &dyn Host,
        request: &ChatSend,
    ) -> Result<ChatSent, ChatStop> {
        if !host.source_enabled(crate::SOURCE_CLIENT) || !host.source_enabled(SOURCE_CHAT) {
            return Err(no_chat("英雄联盟插件的「在客户端里发聊天」关着，这次没发"));
        }
        let lcu = match self.connected_lcu(host) {
            Some(lcu) => lcu,
            None => self
                .lcu_or_closed(host)
                .map_err(|_| read_error(LolError::Unavailable))?,
        };
        let result = send(&lcu, request).await;
        log::log(&match &result {
            Ok(sent) => format!(
                "lol: 经客户端发到{}（{} 行）",
                sent.channel_name, sent.sent_lines
            ),
            Err(stop) => format!("lol: 经客户端发聊天没发（{}）", stop.code.as_str()),
        });
        result
    }
}

#[cfg(test)]
mod tests;
