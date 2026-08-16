//! 科大讯飞语音听写（流式 IAT，`wss://iat-api.xfyun.cn/v2/iat`）。
//!
//! 录音过程中建连并按 1280 字节帧推流；首帧带 `dwa=wpgs`，按 `pgs`/`rg`
//! 维护结果缓冲并外发 Partial。松手只发 `status=2` 等最终片，不再回放整段音频。

use std::collections::BTreeMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use chrono::{Datelike, Timelike, Utc, Weekday};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tokio::sync::mpsc as tokio_mpsc;
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use kotone_core::stt::{
    EngineCapabilities, SessionConfig, SttEngine, SttEvent, SttSession, Transcript,
};

use crate::model::IFLYTEK_ASR_STT_ID;
use crate::pcm::pcm_f32_to_s16le;

type HmacSha256 = Hmac<Sha256>;
type IflytekSocket = WebSocket<MaybeTlsStream<TcpStream>>;

const WS_HOST: &str = "iat-api.xfyun.cn";
const WS_PATH: &str = "/v2/iat";
/// 官方建议：16k PCM 每帧 1280 字节 = 40ms。
const FRAME_BYTES: usize = 1280;
/// 我们自己发结束帧，把服务端 VAD 拉到文档上限，避免按住说话时的停顿被判停。
const EOS_MS: u32 = 10_000;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const FINALIZE_WAIT: Duration = Duration::from_secs(8);
const DRAIN_TIMEOUT: Duration = Duration::from_millis(20);
const FINAL_READ_TIMEOUT: Duration = Duration::from_millis(200);

enum IflytekCmd {
    Audio(Vec<u8>),
    Finish,
    Cancel,
}

pub struct IflytekAsrEngine;

impl IflytekAsrEngine {
    fn option(cfg: &SessionConfig, key: &str) -> String {
        cfg.options
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string()
    }

    fn keys_complete(cfg: &serde_json::Map<String, serde_json::Value>) -> bool {
        let has = |key: &str| {
            cfg.get(key)
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.trim().is_empty())
        };
        has("appId") && has("apiKey") && has("apiSecret")
    }
}

impl SttEngine for IflytekAsrEngine {
    fn id(&self) -> &'static str {
        IFLYTEK_ASR_STT_ID
    }

    fn display_name(&self) -> &str {
        "科大讯飞语音听写"
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            streaming: true,
            hotwords: false,
            gpu: false,
            offline: false,
            languages: vec!["zh".into()],
        }
    }

    fn is_ready(&self) -> bool {
        kotone_core::settings::load()
            .model_configs
            .get(IFLYTEK_ASR_STT_ID)
            .and_then(|cfg| cfg.as_object())
            .is_some_and(Self::keys_complete)
    }

    fn start_session(
        &self,
        cfg: &SessionConfig,
        events: tokio_mpsc::UnboundedSender<SttEvent>,
    ) -> Result<Box<dyn SttSession>, String> {
        let app_id = Self::option(cfg, "appId");
        let api_key = Self::option(cfg, "apiKey");
        let api_secret = Self::option(cfg, "apiSecret");
        if app_id.is_empty() || api_key.is_empty() || api_secret.is_empty() {
            return Err("科大讯飞语音听写未就绪：请填写 APPID、APIKey 与 APISecret".into());
        }
        Ok(Box::new(IflytekSession::start(
            app_id, api_key, api_secret, events,
        )?))
    }
}

pub(crate) struct IflytekSession {
    events: tokio_mpsc::UnboundedSender<SttEvent>,
    cancelled: bool,
    cmd_tx: mpsc::Sender<IflytekCmd>,
    result_rx: Option<mpsc::Receiver<Result<String, String>>>,
    io: Option<thread::JoinHandle<()>>,
}

impl IflytekSession {
    pub(crate) fn start(
        app_id: String,
        api_key: String,
        api_secret: String,
        events: tokio_mpsc::UnboundedSender<SttEvent>,
    ) -> Result<Self, String> {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let events_io = events.clone();
        let io = thread::Builder::new()
            .name("iflytek-iat".into())
            .spawn(move || {
                let result = run_session(app_id, api_key, api_secret, cmd_rx, events_io);
                let _ = result_tx.send(result);
            })
            .map_err(|e| format!("无法启动科大讯飞会话：{e}"))?;
        Ok(Self {
            events,
            cancelled: false,
            cmd_tx,
            result_rx: Some(result_rx),
            io: Some(io),
        })
    }
}

impl SttSession for IflytekSession {
    fn push_audio(&mut self, pcm: &[f32]) -> Result<(), String> {
        if self.cancelled {
            return Ok(());
        }
        self.cmd_tx
            .send(IflytekCmd::Audio(pcm_f32_to_s16le(pcm)))
            .map_err(|_| "科大讯飞会话已结束".to_string())
    }

    fn finalize(mut self: Box<Self>) -> Result<Transcript, String> {
        if self.cancelled {
            return Ok(Transcript {
                text: String::new(),
                latency_ms: 0,
            });
        }
        let started = Instant::now();
        let _ = self.cmd_tx.send(IflytekCmd::Finish);
        let result = self
            .result_rx
            .take()
            .expect("result_rx taken")
            .recv_timeout(FINALIZE_WAIT);
        if let Some(handle) = self.io.take() {
            let _ = handle.join();
        }
        match result {
            Ok(Ok(text)) => {
                let latency_ms = started.elapsed().as_millis() as u32;
                let _ = self.events.send(SttEvent::Final {
                    text: text.clone(),
                    latency_ms,
                });
                Ok(Transcript { text, latency_ms })
            }
            Ok(Err(error)) => Err(error),
            Err(_) => Err("科大讯飞语音听写超时".into()),
        }
    }

    fn cancel(&mut self) {
        self.cancelled = true;
        let _ = self.cmd_tx.send(IflytekCmd::Cancel);
    }
}

/// 按 `sn` 存片；`pgs=rpl` 时删掉 `rg` 范围内的旧片再写入本片。
#[derive(Default)]
struct IflytekTranscript {
    parts: BTreeMap<i64, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IflytekSlice {
    sn: i64,
    pgs: Option<String>,
    rg: Option<(i64, i64)>,
    text: String,
}

impl IflytekTranscript {
    fn apply(&mut self, slice: &IflytekSlice) {
        if slice.pgs.as_deref() == Some("rpl") {
            if let Some((from, to)) = slice.rg {
                for sn in from..=to {
                    self.parts.remove(&sn);
                }
            }
        }
        if !slice.text.is_empty() {
            self.parts.insert(slice.sn, slice.text.clone());
        }
    }

    fn text(&self) -> String {
        self.parts.values().cloned().collect()
    }
}

fn rfc1123_gmt(now: chrono::DateTime<Utc>) -> String {
    let weekday = match now.weekday() {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    };
    let month = match now.month() {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        _ => "Dec",
    };
    format!(
        "{weekday}, {:02} {month} {:04} {:02}:{:02}:{:02} GMT",
        now.day(),
        now.year(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

fn iflytek_auth(api_key: &str, api_secret: &str, date: &str) -> String {
    let signature_origin = format!("host: {WS_HOST}\ndate: {date}\nGET {WS_PATH} HTTP/1.1");
    let mut mac = HmacSha256::new_from_slice(api_secret.as_bytes()).expect("HMAC 接受任意长度密钥");
    mac.update(signature_origin.as_bytes());
    let signature = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    let authorization_origin = format!(
        "api_key=\"{api_key}\", algorithm=\"hmac-sha256\", \
         headers=\"host date request-line\", signature=\"{signature}\""
    );
    base64::engine::general_purpose::STANDARD.encode(authorization_origin.as_bytes())
}

fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn auth_url(api_key: &str, api_secret: &str) -> String {
    let date = rfc1123_gmt(Utc::now());
    let authorization = iflytek_auth(api_key, api_secret, &date);
    format!(
        "wss://{WS_HOST}{WS_PATH}?authorization={}&date={}&host={WS_HOST}",
        url_encode(&authorization),
        url_encode(&date),
    )
}

fn iflytek_audio_frame(app_id: &str, status: u8, audio: &[u8]) -> serde_json::Value {
    let audio_b64 = base64::engine::general_purpose::STANDARD.encode(audio);
    let data = serde_json::json!({
        "status": status,
        "format": "audio/L16;rate=16000",
        "encoding": "raw",
        "audio": audio_b64,
    });
    if status == 0 {
        serde_json::json!({
            "common": {"app_id": app_id},
            "business": {
                "language": "zh_cn",
                "domain": "iat",
                "accent": "mandarin",
                "eos": EOS_MS,
                "dwa": "wpgs",
            },
            "data": data,
        })
    } else {
        serde_json::json!({ "data": data })
    }
}

fn extract_words(json: &serde_json::Value) -> String {
    let mut out = String::new();
    let Some(ws) = json.pointer("/data/result/ws").and_then(|v| v.as_array()) else {
        return out;
    };
    for item in ws {
        let Some(cw) = item.get("cw").and_then(|v| v.as_array()) else {
            continue;
        };
        for w in cw {
            if let Some(t) = w.get("w").and_then(|v| v.as_str()) {
                out.push_str(t);
            }
        }
    }
    out
}

fn parse_slice(json: &serde_json::Value) -> Option<IflytekSlice> {
    let result = json.pointer("/data/result")?;
    let sn = result.get("sn").and_then(|v| v.as_i64()).unwrap_or(0);
    let pgs = result
        .get("pgs")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let rg = result.get("rg").and_then(|v| v.as_array()).and_then(|arr| {
        let from = arr.first()?.as_i64()?;
        let to = arr.get(1)?.as_i64()?;
        Some((from, to))
    });
    Some(IflytekSlice {
        sn,
        pgs,
        rg,
        text: extract_words(json),
    })
}

#[derive(Debug)]
struct ServerMsg {
    is_final: bool,
    slice: Option<IflytekSlice>,
}

fn parse_server_msg(payload: &str) -> Result<ServerMsg, String> {
    let json: serde_json::Value =
        serde_json::from_str(payload).map_err(|e| format!("科大讯飞响应不是 JSON：{e}"))?;
    let code = json.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    let message = json
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if code != 0 {
        return Err(format!("科大讯飞语音听写失败（{code}）：{message}"));
    }
    let is_final = json
        .pointer("/data/status")
        .and_then(|v| v.as_u64())
        .is_some_and(|s| s == 2);
    Ok(ServerMsg {
        is_final,
        slice: parse_slice(&json),
    })
}

fn handshake_error(error: tungstenite::Error) -> String {
    match error {
        tungstenite::Error::Http(resp) => {
            let body = resp
                .body()
                .as_deref()
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_default();
            format!("HTTP {}: {body}", resp.status())
        }
        other => other.to_string(),
    }
}

fn connect(api_key: &str, api_secret: &str) -> Result<IflytekSocket, String> {
    let url = auth_url(api_key, api_secret);
    let request = url
        .into_client_request()
        .map_err(|e| format!("科大讯飞 WebSocket 地址无效：{e}"))?;
    let host = request
        .uri()
        .host()
        .ok_or_else(|| "科大讯飞 WebSocket 地址缺少 host".to_string())?
        .to_string();
    let port = request.uri().port_u16().unwrap_or(443);
    let stream = connect_tcp(&host, port)?;
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(CONNECT_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CONNECT_TIMEOUT));
    match tungstenite::client_tls(request, stream) {
        Ok((socket, _)) => Ok(socket),
        Err(tungstenite::HandshakeError::Failure(error)) => {
            let detail = handshake_error(error);
            kotone_core::log::log(&format!("iflytek asr connect error: {detail}"));
            Err(format!("科大讯飞 WebSocket 连接失败：{detail}"))
        }
        Err(other) => {
            kotone_core::log::log(&format!("iflytek asr connect error: {other}"));
            Err(format!("科大讯飞 WebSocket 连接失败：{other}"))
        }
    }
}

fn connect_tcp(host: &str, port: u16) -> Result<TcpStream, String> {
    let addrs = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("科大讯飞地址解析失败：{e}"))?;
    let mut last = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(stream) => return Ok(stream),
            Err(error) => last = Some(error),
        }
    }
    Err(format!(
        "科大讯飞连接超时或失败：{}",
        last.map(|e| e.to_string())
            .unwrap_or_else(|| "无可用地址".into())
    ))
}

fn set_timeouts(socket: &mut IflytekSocket, timeout: Option<Duration>) {
    let tcp = match socket.get_mut() {
        MaybeTlsStream::Plain(s) => Some(s),
        MaybeTlsStream::Rustls(s) => Some(s.get_mut()),
        _ => None,
    };
    if let Some(tcp) = tcp {
        let _ = tcp.set_read_timeout(timeout);
        let _ = tcp.set_write_timeout(timeout);
    }
}

fn send_frame(
    socket: &mut IflytekSocket,
    app_id: &str,
    status: u8,
    audio: &[u8],
) -> Result<(), String> {
    let frame = iflytek_audio_frame(app_id, status, audio);
    socket
        .send(Message::Text(frame.to_string()))
        .map_err(|e| format!("科大讯飞音频发送失败：{e}"))
}

fn apply_text(
    transcript: &mut IflytekTranscript,
    last: &mut String,
    events: &tokio_mpsc::UnboundedSender<SttEvent>,
    slice: IflytekSlice,
) {
    transcript.apply(&slice);
    let text = transcript.text();
    if !text.is_empty() && text != *last {
        *last = text.clone();
        let _ = events.send(SttEvent::Partial { text });
    }
}

fn drain_messages(
    socket: &mut IflytekSocket,
    transcript: &mut IflytekTranscript,
    last: &mut String,
    events: &tokio_mpsc::UnboundedSender<SttEvent>,
) -> Result<bool, String> {
    match socket.read() {
        Ok(Message::Text(payload)) => {
            let msg = parse_server_msg(&payload)?;
            if let Some(slice) = msg.slice {
                apply_text(transcript, last, events, slice);
            }
            Ok(msg.is_final)
        }
        Ok(Message::Close(_)) => Ok(true),
        Ok(_) => Ok(false),
        Err(tungstenite::Error::Io(error))
            if error.kind() == std::io::ErrorKind::WouldBlock
                || error.kind() == std::io::ErrorKind::TimedOut =>
        {
            Ok(false)
        }
        Err(tungstenite::Error::ConnectionClosed) => Ok(true),
        Err(error) => Err(format!("科大讯飞识别响应失败：{error}")),
    }
}

fn run_session(
    app_id: String,
    api_key: String,
    api_secret: String,
    cmd_rx: mpsc::Receiver<IflytekCmd>,
    events: tokio_mpsc::UnboundedSender<SttEvent>,
) -> Result<String, String> {
    let mut socket = connect(&api_key, &api_secret)?;
    set_timeouts(&mut socket, Some(DRAIN_TIMEOUT));

    let mut leftover = Vec::new();
    let mut sent_first = false;
    let mut server_done = false;
    let mut transcript = IflytekTranscript::default();
    let mut last = String::new();

    loop {
        match cmd_rx.recv_timeout(DRAIN_TIMEOUT) {
            Ok(IflytekCmd::Audio(bytes)) => {
                if server_done {
                    continue;
                }
                leftover.extend_from_slice(&bytes);
                while leftover.len() >= FRAME_BYTES {
                    let frame: Vec<u8> = leftover.drain(..FRAME_BYTES).collect();
                    let status = if sent_first { 1 } else { 0 };
                    send_frame(&mut socket, &app_id, status, &frame)?;
                    sent_first = true;
                    if drain_messages(&mut socket, &mut transcript, &mut last, &events)? {
                        server_done = true;
                        leftover.clear();
                        break;
                    }
                }
            }
            Ok(IflytekCmd::Finish) => {
                if server_done {
                    socket.close(None).ok();
                    return Ok(transcript.text());
                }
                if !sent_first && leftover.is_empty() {
                    socket.close(None).ok();
                    return Ok(String::new());
                }
                if !sent_first {
                    send_frame(&mut socket, &app_id, 0, &leftover)?;
                    leftover.clear();
                } else if !leftover.is_empty() {
                    send_frame(&mut socket, &app_id, 1, &leftover)?;
                    leftover.clear();
                }
                send_frame(&mut socket, &app_id, 2, &[])?;
                set_timeouts(&mut socket, Some(FINAL_READ_TIMEOUT));
                let deadline = Instant::now() + FINALIZE_WAIT;
                while Instant::now() < deadline {
                    match drain_messages(&mut socket, &mut transcript, &mut last, &events) {
                        Ok(true) => break,
                        Ok(false) => {}
                        Err(error) => {
                            socket.close(None).ok();
                            return if transcript.text().is_empty() {
                                Err(error)
                            } else {
                                Ok(transcript.text())
                            };
                        }
                    }
                }
                socket.close(None).ok();
                return Ok(transcript.text());
            }
            Ok(IflytekCmd::Cancel) | Err(RecvTimeoutError::Disconnected) => {
                socket.close(None).ok();
                return Ok(String::new());
            }
            Err(RecvTimeoutError::Timeout) => {
                if drain_messages(&mut socket, &mut transcript, &mut last, &events)? {
                    server_done = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn rfc1123_is_english_gmt() {
        let now = Utc.with_ymd_and_hms(2026, 8, 16, 11, 51, 42).unwrap();
        assert_eq!(rfc1123_gmt(now), "Sun, 16 Aug 2026 11:51:42 GMT");
    }

    #[test]
    fn first_frame_uses_eos_and_dwa() {
        let frame = iflytek_audio_frame("app", 0, b"abc");
        assert_eq!(frame["common"]["app_id"], "app");
        assert_eq!(frame["business"]["eos"], EOS_MS);
        assert_eq!(frame["business"]["dwa"], "wpgs");
        assert!(frame["business"].get("vad_eos").is_none());
        assert_eq!(frame["data"]["status"], 0);
        assert_eq!(frame["data"]["encoding"], "raw");
    }

    #[test]
    fn later_frames_are_data_only() {
        let frame = iflytek_audio_frame("app", 1, b"abc");
        assert!(frame.get("common").is_none());
        assert!(frame.get("business").is_none());
        assert_eq!(frame["data"]["status"], 1);
        let end = iflytek_audio_frame("app", 2, b"");
        assert_eq!(end["data"]["status"], 2);
        assert_eq!(end["data"]["audio"], "");
    }

    #[test]
    fn url_encode_escapes_auth_and_date() {
        assert_eq!(
            url_encode("Wed, 10 Jul 2019 07:35:43 GMT"),
            "Wed%2C%2010%20Jul%202019%2007%3A35%3A43%20GMT"
        );
        assert!(url_encode("a+b/c=").contains("%2B"));
        assert!(url_encode("a+b/c=").contains("%2F"));
        assert!(url_encode("a+b/c=").contains("%3D"));
    }

    #[test]
    fn dwa_replaces_and_appends_slices() {
        let mut t = IflytekTranscript::default();
        t.apply(&IflytekSlice {
            sn: 1,
            pgs: Some("apd".into()),
            rg: None,
            text: "语".into(),
        });
        t.apply(&IflytekSlice {
            sn: 2,
            pgs: Some("rpl".into()),
            rg: Some((1, 1)),
            text: "语音".into(),
        });
        t.apply(&IflytekSlice {
            sn: 3,
            pgs: Some("rpl".into()),
            rg: Some((1, 2)),
            text: "语音听写可以将".into(),
        });
        t.apply(&IflytekSlice {
            sn: 4,
            pgs: Some("apd".into()),
            rg: None,
            text: "。".into(),
        });
        assert_eq!(t.text(), "语音听写可以将。");
    }

    #[test]
    fn without_dwa_slices_append_by_sn() {
        let mut t = IflytekTranscript::default();
        t.apply(&IflytekSlice {
            sn: 1,
            pgs: None,
            rg: None,
            text: "语音".into(),
        });
        t.apply(&IflytekSlice {
            sn: 2,
            pgs: None,
            rg: None,
            text: "听写可以将。".into(),
        });
        assert_eq!(t.text(), "语音听写可以将。");
    }

    #[test]
    fn parse_server_msg_reads_dwa_replace() {
        let payload = r#"{
            "code": 0,
            "message": "success",
            "data": {
                "status": 1,
                "result": {
                    "pgs": "rpl",
                    "rg": [1, 2],
                    "sn": 3,
                    "ws": [{"cw": [{"w": "语音听写可以将"}]}]
                }
            }
        }"#;
        let msg = parse_server_msg(payload).unwrap();
        assert!(!msg.is_final);
        let slice = msg.slice.unwrap();
        assert_eq!(slice.sn, 3);
        assert_eq!(slice.pgs.as_deref(), Some("rpl"));
        assert_eq!(slice.rg, Some((1, 2)));
        assert_eq!(slice.text, "语音听写可以将");
    }

    #[test]
    fn parse_server_msg_surfaces_error_code() {
        let err = parse_server_msg(r#"{"code":10200,"message":"read data timeout"}"#).unwrap_err();
        assert!(err.contains("10200"));
        assert!(err.contains("read data timeout"));
    }

    #[test]
    fn keys_complete_checks_iflytek_fields() {
        let cfg: serde_json::Map<_, _> = serde_json::json!({
            "appId": "app", "apiKey": "key", "apiSecret": "sec"
        })
        .as_object()
        .unwrap()
        .clone();
        assert!(IflytekAsrEngine::keys_complete(&cfg));
        let incomplete: serde_json::Map<_, _> = serde_json::json!({
            "appId": "app", "apiKey": "key"
        })
        .as_object()
        .unwrap()
        .clone();
        assert!(!IflytekAsrEngine::keys_complete(&incomplete));
    }

    #[test]
    fn parse_server_msg_final_status() {
        let msg = parse_server_msg(
            r#"{"code":0,"message":"success","data":{"status":2,"result":{"sn":1,"ws":[]}}}"#,
        )
        .unwrap();
        assert!(msg.is_final);
    }
}
