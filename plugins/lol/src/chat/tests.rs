//! 发到游戏（经客户端的聊天接口）：本机的假客户端走真 HTTP（照 `client/assets.rs` 的写法），记下每个请求。

use std::sync::{Arc, Mutex};

use kotone_plugin_api::CancelToken;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

const TOKEN: &str = "tok";
const CHAMP_SELECT_ID: &str = "8f3e2c1a@lol-champ-select.tj100.pvp.net";
const LOBBY_ID: &str = "4a2b9d7e@lol-lobby.tj100.pvp.net";

#[derive(Clone, Copy)]
enum Post {
    Ok,
    Status(u16),
    /// 3 秒以后才回
    Hang,
}

#[derive(Clone)]
struct Fake {
    phase: &'static str,
    /// 选英雄的数据或房间的数据（看是不是自定义）
    data: Value,
    conversations: Value,
    post: Post,
}

impl Fake {
    fn new(phase: &'static str) -> Self {
        Self {
            phase,
            data: json!({"isCustomGame": false, "gameConfig": {"isCustom": false}}),
            conversations: json!([
                {"id": "friend@pvp.net", "type": "chat"},
                {"id": CHAMP_SELECT_ID, "type": "championSelect"},
                {"id": LOBBY_ID, "type": "customGame"},
            ]),
            post: Post::Ok,
        }
    }
}

/// (方法, 路径, 请求体)
type Seen = Arc<Mutex<Vec<(String, String, String)>>>;

async fn read_request(
    socket: &mut tokio::net::TcpStream,
) -> Option<(String, String, String, bool)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let n = socket.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut words = head.split_whitespace();
    let (method, path) = (words.next()?.to_string(), words.next()?.to_string());
    let header = |name: &str| {
        head.lines()
            .find_map(|l| {
                l.split_once(':')
                    .filter(|(k, _)| k.eq_ignore_ascii_case(name))
            })
            .map(|(_, v)| v.trim().to_string())
    };
    let auth =
        header("authorization").as_deref() == Some(format!("Basic {}", base64_token()).as_str());
    let length: usize = header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    while buf.len() < head_end + length {
        let n = socket.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
    Some((method, path, body, auth))
}

fn base64_token() -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(format!("riot:{TOKEN}"))
}

fn reply(fake: &Fake, method: &str, path: &str) -> (u16, Value) {
    match (method, path) {
        ("GET", "/lol-gameflow/v1/gameflow-phase") => (200, json!(fake.phase)),
        ("GET", "/lol-champ-select/v1/session") | ("GET", "/lol-lobby/v2/lobby") => {
            (200, fake.data.clone())
        }
        ("GET", "/lol-chat/v1/conversations") => (200, fake.conversations.clone()),
        ("POST", p) if p.starts_with("/lol-chat/v1/conversations/") && p.ends_with("/messages") => {
            match fake.post {
                Post::Ok | Post::Hang => (200, json!({"id": "1", "type": "chat"})),
                Post::Status(code) => (code, json!({"message": "not allowed"})),
            }
        }
        _ => (404, json!({"message": "no such path"})),
    }
}

async fn serve(fake: Fake) -> (Lcu, Seen) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Seen::default();
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let (fake, log) = (fake.clone(), log.clone());
            tokio::spawn(async move {
                let Some((method, path, body, auth)) = read_request(&mut socket).await else {
                    return;
                };
                log.lock()
                    .unwrap()
                    .push((method.clone(), path.clone(), body));
                let (status, value) = if auth {
                    reply(&fake, &method, &path)
                } else {
                    (401, json!({"message": "bad auth"}))
                };
                if matches!(fake.post, Post::Hang) && method == "POST" {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                }
                let data = value.to_string();
                let head = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    data.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(data.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    (
        Lcu::with_base(&format!("http://127.0.0.1:{port}"), TOKEN),
        seen,
    )
}

fn request(lines: &[&str], channel: &str, within: Duration) -> ChatSend {
    ChatSend {
        game: "lol".into(),
        lines: lines.iter().map(|l| l.to_string()).collect(),
        channel: channel.into(),
        deadline: Instant::now() + within,
        cancel: CancelToken::default(),
    }
}

const SOON: Duration = Duration::from_secs(5);

fn posts(seen: &Seen) -> Vec<(String, String)> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|(m, _, _)| m == "POST")
        .map(|(_, p, b)| (p.clone(), b.clone()))
        .collect()
}

#[tokio::test]
async fn champ_select_sends_one_message_with_the_lines_joined() {
    let (lcu, seen) = serve(Fake::new("ChampSelect")).await;
    let sent = send(&lcu, &request(&["亚索 5/0/2", "劫 0/4/1"], "team", SOON))
        .await
        .unwrap();
    assert_eq!(
        sent,
        ChatSent {
            channel: "team".into(),
            channel_name: "选英雄的聊天".into(),
            sent_lines: 2,
            stopped: None,
        }
    );
    let posts = posts(&seen);
    assert_eq!(posts.len(), 1, "一次发完：{posts:?}");
    assert_eq!(
        posts[0].0,
        format!("/lol-chat/v1/conversations/{CHAMP_SELECT_ID}/messages"),
        "id 原样拼进路径"
    );
    let body: Value = serde_json::from_str(&posts[0].1).unwrap();
    assert_eq!(
        body,
        json!({"body": "亚索 5/0/2\n劫 0/4/1", "type": "chat"})
    );
    // 读的是现在的阶段、选英雄的数据、会话列表
    let reads: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|(m, _, _)| m == "GET")
        .map(|(_, p, _)| p.clone())
        .collect();
    assert_eq!(
        reads,
        [
            "/lol-gameflow/v1/gameflow-phase",
            "/lol-champ-select/v1/session",
            "/lol-chat/v1/conversations"
        ]
    );
}

#[tokio::test]
async fn the_lobby_queueing_and_ready_check_use_the_lobby_chat() {
    for phase in ["Lobby", "Matchmaking", "ReadyCheck"] {
        let (lcu, seen) = serve(Fake::new(phase)).await;
        let sent = send(&lcu, &request(&["我先排位"], "team", SOON))
            .await
            .unwrap();
        assert_eq!(sent.channel_name, "房间的聊天", "{phase}");
        let posts = posts(&seen);
        assert_eq!(
            posts,
            [(
                format!("/lol-chat/v1/conversations/{LOBBY_ID}/messages"),
                json!({"body": "我先排位", "type": "chat"}).to_string()
            )],
            "{phase}"
        );
    }
}

#[tokio::test]
async fn a_custom_game_is_seen_by_both_sides() {
    for (phase, data) in [
        ("Lobby", json!({"gameConfig": {"isCustom": true}})),
        ("ChampSelect", json!({"isCustomGame": true})),
    ] {
        let fake = Fake {
            data,
            ..Fake::new(phase)
        };
        let (lcu, seen) = serve(fake).await;
        let stop = send(&lcu, &request(&["gg"], "team", SOON))
            .await
            .unwrap_err();
        assert_eq!(stop.code, StopCode::NoSuchChannel, "{phase}");
        assert!(stop.message.contains("两边都看得到"), "{stop}");
        assert!(posts(&seen).is_empty(), "要队伍、这里两边都看得到：不发");
        let sent = send(&lcu, &request(&["gg"], "all", SOON)).await.unwrap();
        assert_eq!(sent.channel, "all");
        assert_eq!(posts(&seen).len(), 1);
    }
}

#[tokio::test]
async fn all_in_champ_select_sends_nothing() {
    let (lcu, seen) = serve(Fake::new("ChampSelect")).await;
    let stop = send(&lcu, &request(&["gg"], "all", SOON))
        .await
        .unwrap_err();
    assert_eq!(stop.code, StopCode::NoSuchChannel);
    assert_eq!(
        stop.message,
        "选英雄的聊天只有队友看得到，没有发给所有人的，这次没发"
    );
    assert!(posts(&seen).is_empty());
}

#[tokio::test]
async fn nothing_is_sent_outside_the_lobby_and_champ_select() {
    for (phase, says) in [
        ("None", "进房间或选英雄"),
        ("InProgress", "切到游戏窗口"),
        ("EndOfGame", "结算界面"),
    ] {
        let (lcu, seen) = serve(Fake::new(phase)).await;
        let stop = send(&lcu, &request(&["gg"], "team", SOON))
            .await
            .unwrap_err();
        assert_eq!(stop.code, StopCode::NoChatHere, "{phase}");
        assert!(stop.message.contains(says), "{phase}: {stop}");
        assert!(posts(&seen).is_empty(), "{phase}");
        assert_eq!(seen.lock().unwrap().len(), 1, "只读了阶段：{phase}");
    }
}

#[tokio::test]
async fn a_chat_that_is_not_there_yet_sends_nothing() {
    // 只有一个 id 里没有 lol-champ-select 的 championSelect 会话（照 LeagueAkari 不认）
    let fake = Fake {
        conversations: json!([{"id": "old@champ-select.pvp.net", "type": "championSelect"}]),
        ..Fake::new("ChampSelect")
    };
    let (lcu, seen) = serve(fake).await;
    let stop = send(&lcu, &request(&["gg"], "team", SOON))
        .await
        .unwrap_err();
    assert_eq!(stop.code, StopCode::NoChatHere);
    assert_eq!(stop.message, "选英雄的聊天还没连上，这次没发：过一两秒再发");
    assert!(posts(&seen).is_empty());
}

#[tokio::test]
async fn the_client_saying_no_is_not_retried() {
    for (code, want, says) in [
        (404, StopCode::NoChatHere, "选英雄的聊天已经没了"),
        (500, StopCode::ClientRejected, "返回 500：not allowed"),
    ] {
        let fake = Fake {
            post: Post::Status(code),
            ..Fake::new("ChampSelect")
        };
        let (lcu, seen) = serve(fake).await;
        let stop = send(&lcu, &request(&["gg"], "team", SOON))
            .await
            .unwrap_err();
        assert_eq!(stop.code, want, "{code}");
        assert!(stop.message.contains(says), "{stop}");
        assert_eq!(posts(&seen).len(), 1, "只发一次");
    }
}

#[tokio::test]
async fn a_post_without_an_answer_returns_by_the_deadline_and_says_it_may_have_gone_out() {
    let fake = Fake {
        post: Post::Hang,
        ..Fake::new("ChampSelect")
    };
    let (lcu, seen) = serve(fake).await;
    let started = Instant::now();
    let stop = send(&lcu, &request(&["gg"], "team", Duration::from_millis(900)))
        .await
        .unwrap_err();
    let took = started.elapsed();
    assert_eq!(stop.code, StopCode::ClientNoReply);
    assert!(stop.message.contains("可能已经发出去了"), "{stop}");
    assert!(
        took < Duration::from_millis(1200),
        "截止时间前返回：{took:?}"
    );
    assert_eq!(posts(&seen).len(), 1, "不重发");
}

#[tokio::test]
async fn too_little_time_or_a_cancel_sends_nothing() {
    let (lcu, seen) = serve(Fake::new("ChampSelect")).await;
    let stop = send(&lcu, &request(&["gg"], "team", Duration::from_millis(100)))
        .await
        .unwrap_err();
    assert_eq!(stop.code, StopCode::TimeUp);
    assert!(seen.lock().unwrap().is_empty(), "一个请求都没发");

    let cancelled = request(&["gg"], "team", SOON);
    cancelled.cancel.cancel();
    let stop = send(&lcu, &cancelled).await.unwrap_err();
    assert_eq!(stop.code, StopCode::Cancelled);
    assert!(posts(&seen).is_empty());
}

#[tokio::test]
async fn nobody_listening_is_unavailable() {
    // 1 号端口在测试机上不会有服务（Windows 上连没人听的端口要重试约 2 秒，读阶段的 2 秒先到：报“没回话”）
    let lcu = Lcu::with_base("http://127.0.0.1:1", TOKEN);
    let stop = send(&lcu, &request(&["gg"], "team", SOON))
        .await
        .unwrap_err();
    assert_eq!(stop.code, StopCode::ClientUnavailable);
    assert!(stop.message.contains("这次没发"), "{stop}");
    assert_eq!(
        read_error(LolError::Unavailable).code,
        StopCode::ClientUnavailable
    );
    assert!(read_error(LolError::Unavailable)
        .message
        .contains("连不上英雄联盟客户端"));
}

#[test]
fn places_conversations_and_bodies() {
    assert_eq!(place("ChampSelect").unwrap(), Place::ChampSelect);
    assert_eq!(place("ReadyCheck").unwrap(), Place::Lobby);
    assert!(place("Reconnect").is_err() && place("").is_err());
    let list = json!([
        {"id": "a/b@lol-champ-select", "type": "championSelect"},
        {"id": "ok~1@lol-champ-select.cn", "type": "championSelect"},
    ]);
    assert_eq!(
        conversation(&list, Place::ChampSelect).as_deref(),
        Some("ok~1@lol-champ-select.cn"),
        "带 / 的不拼进路径"
    );
    assert_eq!(conversation(&list, Place::Lobby), None);
    assert_eq!(conversation(&json!({}), Place::Lobby), None);
    assert_eq!(
        message_body(&["一".into()]),
        json!({"body": "一", "type": "chat"})
    );
    assert!(check_channel(Place::Lobby, "team", "team").is_ok());
    assert_eq!(
        check_channel(Place::Lobby, "all", "team")
            .unwrap_err()
            .message,
        "房间的聊天只有房间里的人看得到，没有发给所有人的，这次没发"
    );
}
