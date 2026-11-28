//! 发到游戏（ADR 041）在桌面端的那一面。入口本身（排队、等修饰键、按前台挑配置、频道、逐行发）在
//! `kotone_core::send_to_game`；这里只做桌面端的事：
//!
//! - 全进程只有一份的入口和那一把输入锁（`SharedState.game_chat`，`lib.rs` 建，语音输入经编排器用同一份）；
//! - 本体工具 `game_chat.send`（她：发起方 `agent:<对话 id>`；外部 Agent：`tool-service`）从这里进入口
//!   （旧工具 `companion.type` 和旧陪玩流程的“帮忙打字”以前也走这里，ADR 053 退掉了）；
//! - 在阻塞线程里跑入口，看门狗只防系统调用被安全软件挂住；
//! - 不写执行记录：替玩家发的字不是自动化，她发了什么在那段聊天里看（2026-11-27 玩家，ADR 014 §3 的修订）；
//! - 把内置插件登记的发法（清单 `[[send_to_game]]`，ADR 041 D3）包成入口的 [`ChatProvider`]（[`providers`]）：
//!   前台是插件登记的窗口时入口交给插件（英雄联盟客户端：选英雄、房间里走客户端的聊天接口，不按键）。只从内置插件表建，
//!   第三方插件（`remote_plugins`）拿不到这个能力（D7）。
//!
//! 工具服务调用时带它自己的期限（减一点余量）进来：入口保证期限前返回真实结果——排不上就一个键都不按，
//! 下一行来不及就不开始——不会出现“报超时、其实字发出去了”，外部 Agent 也就不会因此重发（ADR 041 实施记录）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use kotone_core::inject::{CancelToken, InjectError};
use kotone_core::input_lock::InputLock;
use kotone_core::send_to_game::{
    ChatFailure, ChatOutcome, ChatProvider, ChatRequest, ChatSend, ChatSent, ChatStop, GameChat,
    SendRoute, SendToGame, SendToGameSpec, Speaker, StopCode, DEFAULT_DEADLINE,
};
use kotone_plugin_api::Plugin;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::SharedState;

/// 工具服务的期限里留给“把结果送回去”的余量。
const SERVICE_MARGIN: Duration = Duration::from_millis(300);
/// 截止时间之后阻塞线程还没回来，再等这么久就不等了（只有系统调用被挂住时才会走到）。
const WATCHDOG_GRACE: Duration = Duration::from_secs(2);

/// 插件的发法过了截止时间还没回，再等这么久就不等了（插件的规矩是截止前一定返回，这里只防它卡住；比上面的看门狗短）。
const PROVIDER_GRACE: Duration = Duration::from_millis(500);

/// 内置插件登记的发法（清单 `[[send_to_game]]`）→ 入口的发法（`lib.rs` 建入口时交给它）。
pub fn providers(app: &AppHandle) -> Vec<Arc<dyn ChatProvider>> {
    crate::plugins::registry()
        .iter()
        .flat_map(|plugin| {
            plugin.manifest().send_to_game.iter().map(move |spec| {
                Arc::new(PluginChat {
                    app: app.clone(),
                    plugin: plugin.clone(),
                    spec: spec.clone(),
                }) as Arc<dyn ChatProvider>
            })
        })
        .collect()
}

/// 一个内置插件的一项 `[[send_to_game]]`。
struct PluginChat {
    app: AppHandle,
    plugin: Arc<dyn Plugin>,
    spec: SendToGameSpec,
}

impl ChatProvider for PluginChat {
    fn plugin(&self) -> &str {
        &self.plugin.manifest().id
    }

    fn spec(&self) -> &SendToGameSpec {
        &self.spec
    }

    /// 插件关着，或者 `needs` 里有个开关关着：为什么（话和游戏中心的事共用，`Manifest::needs_off`）。
    fn off(&self) -> Option<String> {
        let state = self.app.state::<SharedState>();
        let settings = state.settings.read().unwrap();
        self.plugin
            .manifest()
            .needs_off(&settings, &self.spec.needs)
    }

    /// 入口在阻塞线程里调：把插件的异步方法交给运行时，在这里等结果（用 channel 等，不 `block_on`：哪个线程调都不会
    /// 卡住运行时）。插件过了截止时间还没回，就说可能已经发出去了（不重发）。
    fn send(&self, request: ChatSend) -> Result<ChatSent, ChatStop> {
        let host = crate::plugins::DesktopHost::for_plugin(&self.app, &self.plugin);
        let plugin = self.plugin.clone();
        let wait = request.deadline.saturating_duration_since(Instant::now()) + PROVIDER_GRACE;
        let (tx, rx) = std::sync::mpsc::channel();
        tauri::async_runtime::spawn(async move {
            let _ = tx.send(plugin.send_to_game(host, &request).await);
        });
        rx.recv_timeout(wait).unwrap_or_else(|_| {
            kotone_core::log::log("game_chat: plugin send_to_game did not answer in time");
            Err(ChatStop::new(
                StopCode::ClientNoReply,
                "没在期限内发完：这句可能已经发出去了，先看一眼聊天，别再发一遍",
            ))
        })
    }
}

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
/// `caller` 是发起方（她：`agent:<对话 id>`；工具服务：`tool-service`；只写进日志）；`deadline` 是工具服务的期限。
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

/// 正在经 [`send_to_game_chat`] 发的（她、外部 Agent、热键、插件的一键喊话）的取消：“停下”一起停掉还没发的几行
/// （ADR 041 第 3 期，[`stop_sends`]）。语音输入（玩家自己说的话）、自动化各有自己的取消，不在这里。
fn sending() -> &'static Mutex<Vec<(u64, CancelToken)>> {
    static SENDING: OnceLock<Mutex<Vec<(u64, CancelToken)>>> = OnceLock::new();
    SENDING.get_or_init(Default::default)
}

/// 发完（不管成没成）从 [`sending`] 里拿掉。
struct Sending(u64);

impl Sending {
    fn start(cancel: &CancelToken) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        sending().lock().unwrap().push((id, cancel.clone()));
        Self(id)
    }
}

impl Drop for Sending {
    fn drop(&mut self) {
        sending().lock().unwrap().retain(|(id, _)| *id != self.0);
    }
}

/// “停下”（[`crate::stop::stop_everything`]）：正在发的几行停在下一行之前（已经按下去的不撤回，也不补发）。返回停了几个。
pub fn stop_sends() -> usize {
    let list = sending().lock().unwrap();
    for (_, cancel) in list.iter() {
        cancel.cancel();
    }
    list.len()
}

/// 经入口发（`game_chat.send`、插件的一键喊话）。没发出去返回原因（一个键都没按，或者卡住了）；
/// 发了一部分照实说哪几行没发。“停下”能停掉还没发的行（[`stop_sends`]）。`session` 是发起方，只写进日志。
///
/// 不写执行记录（2026-11-27 玩家：那不是自动化）：她发了什么在那段聊天里她的工具调用上看。
pub async fn send_to_game_chat(
    app: &AppHandle,
    session: &str,
    request: ChatRequest,
) -> Result<Value, String> {
    let send = request.send;
    let _sending = Sending::start(&request.cancel);
    let outcome = send_logged(app, session, request)
        .await
        .map_err(|failure| failure.to_string())?;
    Ok(result_json(&outcome, send))
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

/// 发，并把结果记进日志（不记文字）。`from` 是发起方（`agent:<对话 id>`、`tool-service`、`hotkey`、`plugin:<id>`……）。
async fn send_logged(
    app: &AppHandle,
    from: &str,
    request: ChatRequest,
) -> Result<ChatOutcome, ChatFailure> {
    let speaker = request.speaker;
    let result = send(app, request).await;
    match &result {
        Ok(o) => kotone_core::log::log(&format!(
            "game_chat: {from} {speaker:?} route={} game={} channel={} lines={}/{} stopped={:?}",
            o.route.as_str(),
            o.game,
            o.channel,
            o.sent_lines,
            o.lines.len(),
            o.stopped.as_ref().map(ChatFailure::code)
        )),
        Err(failure) => kotone_core::log::log(&format!(
            "game_chat: {from} {speaker:?} not sent ({})",
            failure.code()
        )),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// “停下”停掉正在经入口发的（ADR 041 第 3 期）：发着的取消掉，发完的不在名单里。
    #[test]
    fn stop_cancels_the_sends_in_flight() {
        let first = CancelToken::default();
        let second = CancelToken::default();
        let a = Sending::start(&first);
        {
            let _b = Sending::start(&second);
        }
        assert!(stop_sends() >= 1);
        assert!(first.is_cancelled(), "还在发的停下");
        assert!(!second.is_cancelled(), "发完了的不碰");
        drop(a);
        assert!(!sending()
            .lock()
            .unwrap()
            .iter()
            .any(|(_, c)| c.is_cancelled()));
    }

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

    /// 内置插件登记的发法（ADR 041 D3）：窗口不重复（入口不用排先后）；第 2 期不能是游戏配置里的进程（还没有按键的
    /// 发法，对局里归本体；以后加 `keys` 时删掉这一条）；英雄联盟登记了客户端。
    #[test]
    fn plugins_register_their_own_windows_and_never_a_game_window() {
        use kotone_core::profile::GameProfile;
        let games: Vec<String> = [
            GameProfile::builtin_lol(),
            GameProfile::builtin_dota2(),
            GameProfile::builtin_cs2(),
        ]
        .into_iter()
        .flat_map(|g| g.process_names)
        .collect();
        let mut seen: Vec<String> = Vec::new();
        for plugin in crate::plugins::registry() {
            let m = plugin.manifest();
            for spec in &m.send_to_game {
                assert!(m.games.contains(&spec.game), "{} {}", m.id, spec.game);
                for window in &spec.windows {
                    let w = window.to_ascii_lowercase();
                    assert!(!seen.contains(&w), "{window} 登记了两次");
                    assert!(
                        !games.iter().any(|g| g.eq_ignore_ascii_case(window)),
                        "{} 登记了游戏本身的窗口 {window}",
                        m.id
                    );
                    seen.push(w);
                }
            }
        }
        assert!(seen.contains(&"leagueclientux.exe".to_string()), "{seen:?}");
    }

    #[test]
    fn the_service_deadline_keeps_a_margin_and_never_lies_in_the_past() {
        let now = Instant::now();
        let later = now + Duration::from_secs(5);
        assert_eq!(service_deadline(later), later - SERVICE_MARGIN);
        assert!(service_deadline(now) >= now);
    }
}
