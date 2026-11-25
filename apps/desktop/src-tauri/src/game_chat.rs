//! 发到游戏（ADR 041）在桌面端的那一面。入口本身（排队、等修饰键、按前台挑配置、频道、逐行发）在
//! `kotone_core::send_to_game`；这里只做桌面端的事：
//!
//! - 全进程只有一份的入口和那一把输入锁（`SharedState.game_chat`，`lib.rs` 建，语音输入经编排器用同一份）；
//! - 本体工具 `game_chat.send`（她：发起方 `agent:<对话 id>`；外部 Agent：`tool-service`）、旧工具
//!   `companion.type`、旧陪玩流程的“帮忙打字”都从这里进入口；
//! - 在阻塞线程里跑入口，看门狗只防系统调用被安全软件挂住；
//! - `game_chat.send` 写执行记录（「自动化 → 记录」）。
//!
//! 工具服务调用时带它自己的期限（减一点余量）进来：入口保证期限前返回真实结果——排不上就一个键都不按，
//! 下一行来不及就不开始——不会出现“报超时、其实字发出去了”，外部 Agent 也就不会因此重发（ADR 041 实施记录）。

use std::sync::Arc;
use std::time::{Duration, Instant};

use kotone_core::inject::InjectError;
use kotone_core::input_lock::InputLock;
use kotone_core::send_to_game::{
    ChatFailure, ChatOutcome, ChatRequest, GameChat, SendRoute, SendToGame, Speaker,
    DEFAULT_DEADLINE,
};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::SharedState;

/// 工具服务的期限里留给“把结果送回去”的余量。
const SERVICE_MARGIN: Duration = Duration::from_millis(300);
/// 截止时间之后阻塞线程还没回来，再等这么久就不等了（只有系统调用被挂住时才会走到）。
const WATCHDOG_GRACE: Duration = Duration::from_secs(2);

/// 宿主那一份入口。
pub fn entry(app: &AppHandle) -> Arc<GameChat> {
    app.state::<SharedState>().game_chat.clone()
}

/// 宿主那一把输入锁（`game.act`、按键程序用）。
pub fn input_lock(app: &AppHandle) -> Arc<InputLock> {
    entry(app).lock().clone()
}

/// 工具服务的期限 → 入口的截止时间（留出送回结果的余量）。
pub fn service_deadline(deadline: Instant) -> Instant {
    deadline
        .checked_sub(SERVICE_MARGIN)
        .unwrap_or(deadline)
        .max(Instant::now())
}

/// 在阻塞线程里经入口发。截止时间之后再等 2 秒线程还没回来，就说卡住了（聊天框里可能有一部分字）。
pub async fn send(app: &AppHandle, request: ChatRequest) -> Result<ChatOutcome, ChatFailure> {
    let entry = entry(app);
    let deadline = request
        .deadline
        .unwrap_or_else(|| Instant::now() + DEFAULT_DEADLINE);
    let cancel = request.cancel.clone();
    let task = tauri::async_runtime::spawn_blocking(move || entry.send(request));
    let wait = deadline.saturating_duration_since(Instant::now()) + WATCHDOG_GRACE;
    match tokio::time::timeout(wait, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(ChatFailure::Inject(InjectError::new("打字线程异常"))),
        Err(_) => {
            cancel.cancel();
            Err(ChatFailure::Inject(InjectError::new(
                "打字没有按时结束：模拟输入可能被安全软件或游戏拦住了，聊天框里可能已经有一部分字，请先看一眼",
            )))
        }
    }
}

/// 回执：按键的只证明本机按键已执行；经客户端接口的只证明客户端收下了（ADR 041 D6）。
fn receipt(outcome: &ChatOutcome, send: bool) -> String {
    let (n, total) = (outcome.sent_lines, outcome.lines.len());
    match (outcome.route, send) {
        (SendRoute::Client, _) => format!(
            "已经经客户端的聊天接口发到{}（{n}/{total} 行{}）；只证明客户端收下了，不证明队友已经看到",
            outcome.channel_name,
            if total > 1 { "，连成一条" } else { "" }
        ),
        (_, false) => {
            format!("已打进聊天框，未按发送键（{n}/{total} 行）；只证明本机按键已执行，不证明服务器已收到")
        }
        (SendRoute::Console, true) => format!(
            "已经由控制台发到{}（{n}/{total} 行）；只证明本机按键已执行，不证明服务器已收到",
            outcome.channel_name
        ),
        (SendRoute::Keys, true) => format!(
            "已按发送键发到{}（{n}/{total} 行）；只证明本机按键已执行，不证明服务器已收到",
            outcome.channel_name
        ),
    }
}

/// 本体用 ToolSpec 登记的工具（[`kotone_agent::body`]，ADR 039 D2）在桌面端的实现：她
/// （`BuiltinHost::body_tool`）和外部 Agent（工具服务、MCP，`tool_config::invoke_until`）都走这里。
/// `caller` 是执行记录的发起方（她：`agent:<对话 id>`；工具服务：`tool-service`）；`deadline` 是工具服务的期限。
pub async fn body_tool(
    app: &AppHandle,
    tool: &str,
    arguments: &Value,
    caller: &str,
    speaker: Speaker,
    deadline: Option<Instant>,
) -> Result<kotone_plugin_api::ToolResult, String> {
    match tool {
        kotone_agent::body::TOOL_GAME_CHAT => {
            let auto_send = app
                .state::<SharedState>()
                .settings
                .read()
                .unwrap()
                .assistant
                .companion
                .auto_send;
            let mut request = kotone_agent::body::chat_request(arguments, auto_send, speaker)?;
            request.deadline = deadline.map(service_deadline);
            send_to_game_chat(app, caller, request)
                .await
                .map(kotone_plugin_api::ToolResult::content)
        }
        _ => Err(format!("本体没有 {tool} 这个工具")),
    }
}

/// 经入口发，并写一条执行记录（`game_chat.send`）。没发出去返回原因（一个键都没按，或者卡住了）；
/// 发了一部分照实说哪几行没发。
pub async fn send_to_game_chat(
    app: &AppHandle,
    session: &str,
    request: ChatRequest,
) -> Result<Value, String> {
    let text = request.text.clone();
    let send = request.send;
    let result = send_logged(app, request).await;
    let (profile, sent, recorded) = match &result {
        Ok(outcome) => (
            outcome.game.clone(),
            send && outcome.sent_lines > 0,
            match &outcome.stopped {
                None => Ok(()),
                Some(stopped) => Err(format!(
                    "发了 {} 行，后面 {} 行没发：{stopped}",
                    outcome.sent_lines,
                    outcome.lines.len() - outcome.sent_lines
                )),
            },
        ),
        Err(failure) => (selected_game(app), false, Err(failure.to_string())),
    };
    let record = crate::automation::record_chat(
        app,
        session,
        kotone_core::automation::ChatAction {
            text,
            sent,
            profile,
        },
        recorded,
    );
    if let Err(e) = &record {
        kotone_core::log::log(&format!("game_chat: chat record failed: {e}"));
    }
    let outcome = result.map_err(|failure| failure.to_string())?;
    let mut value = result_json(&outcome, send);
    if let Ok(run) = record {
        value["runId"] = json!(run.id);
    }
    Ok(value)
}

/// 给她 / 外部 Agent 的结果：以前的 `{typed, sent, profile, receipt}` 照旧（外部 Agent、脚本认这几个字段），
/// 再加上入口的 `route`、`channel`、`lines`、`sentLines`、`stopped`……（只加不改）。`sent` 照旧是“按了发送键”：
/// 只打进聊天框（`send = false`）时是 false；多行时要每一行都发了才是 true。
fn result_json(outcome: &ChatOutcome, send: bool) -> Value {
    let mut value = outcome.to_json();
    value["typed"] = json!(outcome.lines.join("\n"));
    value["sent"] = json!(send && outcome.sent);
    value["profile"] = json!(outcome.game);
    value["receipt"] = json!(receipt(outcome, send));
    value
}

/// 旧工具 `companion.type`、旧陪玩流程：只收一行，结果同 [`result_json`]，不写执行记录。
pub async fn type_into_chat(
    app: &AppHandle,
    text: &str,
    send: bool,
    speaker: Speaker,
    deadline: Option<Instant>,
) -> Result<Value, String> {
    let text = text.trim();
    if text.contains(['\r', '\n']) {
        // 旧接口只收一行；多行用 game_chat.send
        return Err("要打的字含有换行：多行请用 game_chat.send".into());
    }
    let mut request = ChatRequest::new(text, speaker);
    request.send = send;
    request.deadline = deadline.map(service_deadline);
    let outcome = send_logged(app, request)
        .await
        .map_err(|failure| failure.to_string())?;
    Ok(result_json(&outcome, send))
}

/// 发，并把结果记进日志（不记文字）。
async fn send_logged(app: &AppHandle, request: ChatRequest) -> Result<ChatOutcome, ChatFailure> {
    let speaker = request.speaker;
    let result = send(app, request).await;
    match &result {
        Ok(o) => kotone_core::log::log(&format!(
            "game_chat: {speaker:?} route={} game={} channel={} lines={}/{} stopped={:?}",
            o.route.as_str(),
            o.game,
            o.channel,
            o.sent_lines,
            o.lines.len(),
            o.stopped.as_ref().map(ChatFailure::code)
        )),
        Err(failure) => kotone_core::log::log(&format!(
            "game_chat: {speaker:?} not sent ({})",
            failure.code()
        )),
    }
    result
}

/// 当前选中的游戏配置 id（没发出去时记录里写它，和以前一样）。
fn selected_game(app: &AppHandle) -> String {
    let settings = app.state::<SharedState>().settings.read().unwrap().clone();
    crate::companion::active_profile(&settings).id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(lines: &[&str], sent_lines: usize) -> ChatOutcome {
        ChatOutcome {
            route: SendRoute::Keys,
            plugin: None,
            game: "lol".into(),
            game_name: "League of Legends".into(),
            channel: "team".into(),
            channel_name: "队伍".into(),
            channel_fell_back: false,
            lines: lines.iter().map(|l| l.to_string()).collect(),
            sent_lines,
            sent: sent_lines == lines.len(),
            stopped: (sent_lines < lines.len()).then_some(ChatFailure::ForegroundChanged),
            waited_ms: 0,
        }
    }

    /// 以前的结果字段（`typed`、`sent`、`profile`、`receipt`）一个不少，入口的字段是加上去的（单向门）。
    #[test]
    fn the_result_keeps_the_old_fields_and_adds_the_new_ones() {
        let v = result_json(&outcome(&["我去下路"], 1), true);
        assert_eq!(v["typed"], "我去下路");
        assert_eq!(v["sent"], true);
        assert_eq!(v["profile"], "lol");
        assert!(v["receipt"].as_str().unwrap().contains("发到队伍"));
        assert_eq!(
            (v["route"].as_str(), v["channel"].as_str()),
            (Some("keys"), Some("team"))
        );
        assert_eq!(
            (v["lines"].as_u64(), v["sentLines"].as_u64()),
            (Some(1), Some(1))
        );
        // 只打进聊天框：sent 照旧是 false（没按发送键）
        let v = result_json(&outcome(&["先别发"], 1), false);
        assert_eq!(v["sent"], false);
        assert!(v["receipt"].as_str().unwrap().starts_with("已打进聊天框"));
        // 多行发了一半：sent 是 false，stopped 说哪几行没发
        let v = result_json(&outcome(&["一", "二"], 1), true);
        assert_eq!(v["typed"], "一\n二");
        assert_eq!(v["sent"], false);
        assert_eq!(v["stopped"]["code"], "foregroundChanged");
    }

    /// 走插件的发法（英雄联盟客户端的聊天接口，ADR 041 第 2 期）：回执说清楚只证明客户端收下了，带上 plugin。
    #[test]
    fn the_client_route_says_what_its_receipt_proves() {
        let mut out = outcome(&["亚索 5/0/2", "劫 0/4/1"], 2);
        out.route = SendRoute::Client;
        out.plugin = Some("lol".into());
        out.channel_name = "选英雄的聊天".into();
        let v = result_json(&out, true);
        assert_eq!(v["route"], "client");
        assert_eq!(v["plugin"], "lol");
        assert_eq!(v["sent"], true);
        assert_eq!(
            v["receipt"],
            "已经经客户端的聊天接口发到选英雄的聊天（2/2 行，连成一条）；只证明客户端收下了，不证明队友已经看到"
        );
        // 按键的回执一字不变
        let v = result_json(&outcome(&["gg"], 1), true);
        assert_eq!(
            v["receipt"],
            "已按发送键发到队伍（1/1 行）；只证明本机按键已执行，不证明服务器已收到"
        );
        assert!(v.get("plugin").is_none());
    }

    #[test]
    fn the_service_deadline_keeps_a_margin_and_never_lies_in_the_past() {
        let now = Instant::now();
        let later = now + Duration::from_secs(5);
        assert_eq!(service_deadline(later), later - SERVICE_MARGIN);
        assert!(service_deadline(now) >= now);
    }
}
