//! DeepSeek 流式语音合成（WebSocket）：分段发文字，边收 PCM。
//!
//! 协议照 DeepSeek 官方文档（<https://api-docs.deepseek.com/guides/tts>）：
//! - 地址 `wss://api.deepseek.com/v1/tools/tts?voice_id=…&format=pcm&filter_markdown=true`，
//!   请求头 `Authorization: Bearer <key>`；连上后不用发配置；
//! - 客户端文字帧 `{"event":"text","text":"…"}` 可以发多次，最后发 `{"event":"finish_text"}`；
//! - 服务端二进制帧是裸 PCM（16 位有符号、单声道、24 kHz；文档没写字节序，按小端解，
//!   真机测试里核对过），文字帧 `{"event":"finish","code":"success"|…,"message":"…"}` 结束；
//!   没有分句事件；
//! - 一次会话最多 1 万个计费字符、64 KiB 文字；60 秒没有新文字服务端会结束会话；
//!   最长 30 分钟；每个账号最多 50 路并发。
//!
//! 同步 tungstenite（与 kotone-stt 讯飞、火山流式识别同一个库和版本）放在一个专用线程里：
//! 读带 20 ms 超时，没有数据时顺便把排队的文字发出去，所以收、发在同一个线程里交替进行，
//! 不需要拆分连接。取消时立刻关掉连接。
//!
//! 用量：线程退出时为**实际发出去的文字**记一次（`kotone_core::usage::record_tts`，汉字算
//! 2 个计费字符）。密钥只在请求头里，不进日志、错误消息。

use std::{
    io::ErrorKind,
    net::{TcpStream, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use tungstenite::{client::IntoClientRequest, stream::MaybeTlsStream, Message, WebSocket};

use crate::online::{deepseek_origin, OnlineConfig};
use crate::voice;

/// DeepSeek 流式合成的采样率（单声道 16 位）。
pub const SAMPLE_RATE: u32 = 24_000;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 读一次最多等这么久，然后去看有没有要发的文字、有没有取消。
const POLL: Duration = Duration::from_millis(20);
/// 发了 finish_text 以后，服务端这么久什么也不发就算超时。
const FINISH_TIMEOUT: Duration = Duration::from_secs(20);

/// 服务端发来的东西。`Finished` / `Rejected` / `Failed` 之后不会再有事件；取消时什么也不发。
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// 一块 PCM：16 位有符号、单声道、[`SAMPLE_RATE`]。
    Audio(Vec<i16>),
    /// 服务端说全部合成完了（`code == "success"`）。
    Finished,
    /// 这段文字本身不能念（`content_filter` / `invalid_input`）：换一段文字可能就行。
    Rejected(String),
    /// 连接、鉴权、超时、服务出错等（已经是给玩家看的中文）。
    Failed(String),
}

/// 会话怎么结束的。
enum Ended {
    Success,
    Cancelled,
    Rejected(String),
}

enum Command {
    Text(String),
    Finish,
}

/// 一次 DeepSeek 流式合成会话。创建时就在后台连接（把握手藏在模型生成第一句的时间里）。
pub struct DeepseekStream {
    commands: mpsc::Sender<Command>,
    cancelled: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl DeepseekStream {
    /// 连接并开始会话。`sink` 在后台线程里收到每个事件（不要在里面阻塞太久）。
    /// 声音用 `config.voice`（可带 `deepseek/` 前缀）；`voice_override` 非空时优先。
    pub fn open(
        config: &OnlineConfig,
        voice_override: Option<&str>,
        sink: impl FnMut(StreamEvent) + Send + 'static,
    ) -> Result<Self, String> {
        let url = stream_url(config, voice_override)?;
        let key = config.api_key.trim().to_string();
        if key.is_empty() {
            return Err("DeepSeek 连接还没有保存 Key".into());
        }
        let model = config.model.clone();
        let (commands, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let worker = std::thread::Builder::new()
            .name("kotone-deepseek-tts".into())
            .spawn(move || run(url, key, model, receiver, flag, sink))
            .map_err(|e| format!("无法启动语音合成线程：{e}"))?;
        Ok(Self {
            commands,
            cancelled,
            worker: Some(worker),
        })
    }

    /// 同 [`open`](Self::open)，事件放进通道。
    pub fn open_channel(
        config: &OnlineConfig,
        voice_override: Option<&str>,
    ) -> Result<(Self, mpsc::Receiver<StreamEvent>), String> {
        let (tx, rx) = mpsc::channel();
        let stream = Self::open(config, voice_override, move |event| {
            let _ = tx.send(event);
        })?;
        Ok((stream, rx))
    }

    /// 排队发一段文字（连上以后按顺序发出）。空白段不发。
    pub fn send_text(&self, text: &str) {
        if !text.trim().is_empty() {
            let _ = self.commands.send(Command::Text(text.to_string()));
        }
    }

    /// 文字发完了：服务端念完剩下的就发 `finish`。
    pub fn finish_text(&self) {
        let _ = self.commands.send(Command::Finish);
    }

    /// 立刻关闭连接，之后不再有事件。不等后台线程。
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// 等后台线程退出（取消或结束后用；用量在它退出前记好）。
    pub fn join(mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for DeepseekStream {
    fn drop(&mut self) {
        // 丢掉句柄 = 不要了：后台线程看到通道断开或取消标记就关连接。
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

/// `wss://<站点>/v1/tools/tts?voice_id=…&format=pcm&filter_markdown=true`。
/// 连接地址是 `http://` 时（本地测试）用 `ws://`。
pub fn stream_url(config: &OnlineConfig, voice_override: Option<&str>) -> Result<String, String> {
    let chosen = voice_override
        .filter(|v| !v.trim().is_empty())
        .unwrap_or(&config.voice);
    let native = voice::native(voice::DEEPSEEK, chosen)?.trim();
    let native = if native.is_empty() { "stella" } else { native };
    if !native
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(format!("DeepSeek 声音名无效：{native}"));
    }
    let origin = deepseek_origin(&config.base_url);
    let ws = if let Some(rest) = origin.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = origin.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        return Err("DeepSeek 连接地址应以 https:// 开头".into());
    };
    Ok(format!(
        "{ws}/v1/tools/tts?voice_id={native}&format=pcm&filter_markdown=true"
    ))
}

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

fn run(
    url: String,
    key: String,
    model: String,
    commands: mpsc::Receiver<Command>,
    cancelled: Arc<AtomicBool>,
    mut sink: impl FnMut(StreamEvent),
) {
    let mut sent = String::new();
    let result = session(&url, &key, &commands, &cancelled, &mut sink, &mut sent);
    drop(key);
    if !sent.is_empty() {
        kotone_core::usage::record_tts(voice::DEEPSEEK, &model, &sent);
    }
    match result {
        Ok(Ended::Success) => sink(StreamEvent::Finished),
        Ok(Ended::Cancelled) => {}
        Ok(Ended::Rejected(error)) => {
            kotone_core::log::log(&format!("deepseek tts stream: {error}"));
            sink(StreamEvent::Rejected(error));
        }
        Err(error) => {
            if !cancelled.load(Ordering::SeqCst) {
                kotone_core::log::log(&format!("deepseek tts stream: {error}"));
                sink(StreamEvent::Failed(error));
            }
        }
    }
}

fn session(
    url: &str,
    key: &str,
    commands: &mpsc::Receiver<Command>,
    cancelled: &AtomicBool,
    sink: &mut impl FnMut(StreamEvent),
    sent: &mut String,
) -> Result<Ended, String> {
    let mut socket = connect(url, key)?;
    if cancelled.load(Ordering::SeqCst) {
        let _ = socket.close(None);
        return Ok(Ended::Cancelled);
    }
    set_timeouts(&mut socket, Some(POLL), Some(CONNECT_TIMEOUT));
    let mut finish_sent_at: Option<Instant> = None;
    let mut last_server = Instant::now();
    let mut odd_byte: Option<u8> = None;
    loop {
        if cancelled.load(Ordering::SeqCst) {
            let _ = socket.close(None);
            return Ok(Ended::Cancelled);
        }
        // 先把排队的文字发出去
        loop {
            match commands.try_recv() {
                Ok(Command::Text(text)) if finish_sent_at.is_none() => {
                    socket
                        .send(Message::Text(
                            json!({"event": "text", "text": text}).to_string().into(),
                        ))
                        .map_err(|e| format!("DeepSeek 语音合成发送文字失败：{e}"))?;
                    sent.push_str(&text);
                }
                Ok(Command::Text(_)) => {}
                Ok(Command::Finish) => {
                    if finish_sent_at.is_none() {
                        socket
                            .send(Message::Text(
                                json!({"event": "finish_text"}).to_string().into(),
                            ))
                            .map_err(|e| format!("DeepSeek 语音合成发送结束标记失败：{e}"))?;
                        finish_sent_at = Some(Instant::now());
                        last_server = Instant::now();
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                // 句柄被丢掉 = 不要了
                Err(mpsc::TryRecvError::Disconnected) => {
                    let _ = socket.close(None);
                    return Ok(Ended::Cancelled);
                }
            }
        }
        match socket.read() {
            Ok(Message::Binary(bytes)) => {
                last_server = Instant::now();
                let samples = pcm_le(&bytes, &mut odd_byte);
                if !samples.is_empty() {
                    sink(StreamEvent::Audio(samples));
                }
            }
            Ok(Message::Text(text)) => {
                last_server = Instant::now();
                if let Some(done) = parse_finish(&text)? {
                    let _ = socket.close(None);
                    return Ok(done);
                }
            }
            Ok(Message::Close(frame)) => {
                return Err(match frame {
                    Some(frame) => format!(
                        "DeepSeek 语音合成连接被服务端关闭（{}）",
                        u16::from(frame.code)
                    ),
                    None => "DeepSeek 语音合成连接被服务端关闭".into(),
                });
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                if let Some(at) = finish_sent_at {
                    if last_server.max(at).elapsed() > FINISH_TIMEOUT {
                        return Err("DeepSeek 语音合成超时：服务端很久没有返回声音".into());
                    }
                }
            }
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                return Err("DeepSeek 语音合成连接意外断开".into());
            }
            Err(error) => return Err(format!("DeepSeek 语音合成连接出错：{error}")),
        }
    }
}

/// `{"event":"finish","code":…}` → 成功 / 这段文字不能念 / Err 其他错误码；其他事件 None。
fn parse_finish(text: &str) -> Result<Option<Ended>, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|_| "DeepSeek 语音合成返回了无法识别的消息")?;
    if value.get("event").and_then(Value::as_str) != Some("finish") {
        return Ok(None);
    }
    let code = value.get("code").and_then(Value::as_str).unwrap_or("");
    if code == "success" {
        return Ok(Some(Ended::Success));
    }
    // 服务端的 message 只记日志，不回显给玩家（与 online.rs 一致）。
    if let Some(message) = value.get("message").and_then(Value::as_str) {
        let short: String = message.chars().take(200).collect();
        kotone_core::log::log(&format!("deepseek tts stream finish {code}: {short}"));
    }
    match code {
        "content_filter" | "invalid_input" => Ok(Some(Ended::Rejected(finish_error(code)))),
        _ => Err(finish_error(code)),
    }
}

/// 错误码 → 玩家看的话（码表见官方文档）。
pub fn finish_error(code: &str) -> String {
    match code {
        "invalid_input" => "DeepSeek 语音合成拒绝了这段文字（格式或长度不符合要求）".into(),
        "content_filter" => "DeepSeek 语音合成因内容审核没有念这段话".into(),
        "input_timeout" => "DeepSeek 语音合成等文字超时（60 秒没有新文字）".into(),
        "service_error" => "DeepSeek 语音合成服务出错，请稍后再试".into(),
        other => format!("DeepSeek 语音合成失败（{other}）"),
    }
}

/// 小端 16 位 PCM → 样本。二进制帧可能把一个样本拆在两帧之间：多出的一个字节留到下一帧。
fn pcm_le(bytes: &[u8], odd_byte: &mut Option<u8>) -> Vec<i16> {
    let mut out = Vec::with_capacity(bytes.len() / 2 + 1);
    let mut rest = bytes;
    if let Some(low) = odd_byte.take() {
        match rest.split_first() {
            Some((high, tail)) => {
                out.push(i16::from_le_bytes([low, *high]));
                rest = tail;
            }
            None => {
                *odd_byte = Some(low);
                return out;
            }
        }
    }
    let mut pairs = rest.chunks_exact(2);
    out.extend(pairs.by_ref().map(|p| i16::from_le_bytes([p[0], p[1]])));
    if let [last] = pairs.remainder() {
        *odd_byte = Some(*last);
    }
    out
}

fn connect(url: &str, key: &str) -> Result<Socket, String> {
    let mut request = url
        .into_client_request()
        .map_err(|e| format!("DeepSeek 语音合成地址无效：{e}"))?;
    let headers = request.headers_mut();
    headers.insert(
        "Authorization",
        format!("Bearer {key}")
            .parse()
            .map_err(|_| "DeepSeek Key 含有无法放进请求头的字符".to_string())?,
    );
    headers.insert(
        "User-Agent",
        concat!("Kotone/", env!("CARGO_PKG_VERSION"))
            .parse()
            .expect("static header"),
    );
    let uri = request.uri();
    let host = uri
        .host()
        .ok_or("DeepSeek 语音合成地址缺少主机名")?
        .to_string();
    let port = uri
        .port_u16()
        .unwrap_or(if uri.scheme_str() == Some("wss") {
            443
        } else {
            80
        });
    let stream = connect_tcp(&host, port)?;
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(CONNECT_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CONNECT_TIMEOUT));
    match tungstenite::client_tls(request, stream) {
        Ok((socket, _)) => Ok(socket),
        Err(tungstenite::HandshakeError::Failure(tungstenite::Error::Http(response))) => {
            let status = response.status().as_u16();
            kotone_core::log::log(&format!("deepseek tts stream handshake: HTTP {status}"));
            Err(match status {
                401 | 403 => "DeepSeek Key 无效或没有语音合成权限".into(),
                402 => "DeepSeek 账户余额不足".into(),
                429 => "DeepSeek 语音合成请求太多（同时最多 50 路），请稍后再试".into(),
                _ => format!("DeepSeek 语音合成连接失败（HTTP {status}）"),
            })
        }
        Err(error) => Err(format!("DeepSeek 语音合成连接失败：{error}")),
    }
}

fn connect_tcp(host: &str, port: u16) -> Result<TcpStream, String> {
    let addrs = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("DeepSeek 地址解析失败：{e}"))?;
    let mut last = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(stream) => return Ok(stream),
            Err(error) => last = Some(error),
        }
    }
    Err(format!(
        "DeepSeek 语音合成连接超时或失败：{}",
        last.map(|e| e.to_string())
            .unwrap_or_else(|| "没有可用地址".into())
    ))
}

fn set_timeouts(socket: &mut Socket, read: Option<Duration>, write: Option<Duration>) {
    let tcp = match socket.get_mut() {
        MaybeTlsStream::Plain(s) => Some(s),
        MaybeTlsStream::Rustls(s) => Some(s.get_mut()),
        _ => None,
    };
    if let Some(tcp) = tcp {
        let _ = tcp.set_read_timeout(read);
        let _ = tcp.set_write_timeout(write);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::Mutex;
    use tungstenite::handshake::server::{Request, Response};

    /// 本地假服务端收到的东西（所有连接合在一起）。
    #[derive(Default, Debug)]
    pub(crate) struct Seen {
        pub connections: usize,
        pub authorization: String,
        pub query: String,
        pub texts: Vec<String>,
        pub finished_text: usize,
        pub closed_early: usize,
    }

    /// 假服务端怎么回应。
    #[derive(Clone, Copy)]
    pub(crate) struct Script {
        /// 每段文字回 `samples_per_char × 字数` 个样本，样本值 = 字数 × 100
        /// （拆成两帧，故意在样本中间拆开）。
        pub samples_per_char: usize,
        /// 回声音前每个字先等这么久（模拟合成耗时：长句慢）。
        pub delay_per_char: Duration,
        /// 收到 finish_text 后回的码。
        pub finish_code: &'static str,
        /// 含这个词的会话在结束时回 content_filter。
        pub filter_word: Option<&'static str>,
        /// 握手直接回 HTTP 401。
        pub reject: bool,
    }

    impl Script {
        pub(crate) fn echo(samples_per_char: usize) -> Self {
            Self {
                samples_per_char,
                delay_per_char: Duration::ZERO,
                finish_code: "success",
                filter_word: None,
                reject: false,
            }
        }
    }

    /// 起一个本地 WebSocket 服务端（每个连接一个线程），返回 `http://127.0.0.1:端口`。
    pub(crate) fn fake_server(script: Script) -> (String, Arc<Mutex<Seen>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Seen::default()));
        let record = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let record = record.clone();
                std::thread::spawn(move || serve(stream, script, record));
            }
        });
        (base, seen)
    }

    // 握手回调的错误类型是 tungstenite 定的（整个 HTTP 响应），没法变小。
    #[allow(clippy::result_large_err)]
    fn serve(stream: TcpStream, script: Script, record: Arc<Mutex<Seen>>) {
        let callback = |request: &Request, response: Response| {
            let mut seen = record.lock().unwrap();
            seen.connections += 1;
            seen.authorization = request
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            seen.query = request.uri().query().unwrap_or_default().to_string();
            if script.reject {
                let mut reject = tungstenite::http::Response::new(Some("bad key".into()));
                *reject.status_mut() = tungstenite::http::StatusCode::UNAUTHORIZED;
                return Err(reject);
            }
            Ok(response)
        };
        let Ok(mut socket) = tungstenite::accept_hdr(stream, callback) else {
            return;
        };
        let mut filtered = false;
        loop {
            let message = match socket.read() {
                Ok(message) => message,
                Err(_) => {
                    record.lock().unwrap().closed_early += 1;
                    return;
                }
            };
            let Message::Text(text) = message else {
                if matches!(message, Message::Close(_)) {
                    record.lock().unwrap().closed_early += 1;
                    return;
                }
                continue;
            };
            let value: Value = serde_json::from_str(&text).unwrap();
            match value["event"].as_str().unwrap() {
                "text" => {
                    let text = value["text"].as_str().unwrap().to_string();
                    record.lock().unwrap().texts.push(text.clone());
                    let chars = text.chars().count();
                    filtered |= script.filter_word.is_some_and(|w| text.contains(w));
                    std::thread::sleep(script.delay_per_char * chars as u32);
                    if filtered || script.samples_per_char == 0 {
                        continue;
                    }
                    let value = (chars * 100) as i16;
                    let bytes: Vec<u8> = (0..chars * script.samples_per_char)
                        .flat_map(|_| value.to_le_bytes())
                        .collect();
                    let (a, b) = bytes.split_at((bytes.len() / 2) | 1);
                    for part in [a, b] {
                        if socket.send(Message::Binary(part.to_vec().into())).is_err() {
                            record.lock().unwrap().closed_early += 1;
                            return;
                        }
                    }
                }
                "finish_text" => {
                    record.lock().unwrap().finished_text += 1;
                    let code = if filtered {
                        "content_filter"
                    } else {
                        script.finish_code
                    };
                    let _ = socket.send(Message::Text(
                        json!({"event": "finish", "code": code, "message": "detail"})
                            .to_string()
                            .into(),
                    ));
                    let _ = socket.close(None);
                    let _ = socket.flush();
                    return;
                }
                other => panic!("unexpected event {other}"),
            }
        }
    }

    pub(crate) fn config(base_url: &str) -> OnlineConfig {
        OnlineConfig {
            base_url: base_url.into(),
            api_key: "sk-test-not-real".into(),
            model: String::new(),
            app_id: String::new(),
            voice: "deepseek/stella".into(),
        }
    }

    fn collect(rx: &mpsc::Receiver<StreamEvent>) -> (Vec<i16>, Option<StreamEvent>) {
        let mut audio = Vec::new();
        loop {
            match rx.recv_timeout(Duration::from_secs(5)) {
                Ok(StreamEvent::Audio(chunk)) => audio.extend(chunk),
                Ok(other) => return (audio, Some(other)),
                Err(_) => return (audio, None),
            }
        }
    }

    #[test]
    fn url_uses_wss_the_site_root_and_the_native_voice() {
        let mut c = config("https://api.deepseek.com/v1/");
        assert_eq!(
            stream_url(&c, None).unwrap(),
            "wss://api.deepseek.com/v1/tools/tts?voice_id=stella&format=pcm&filter_markdown=true"
        );
        assert!(stream_url(&c, Some("tide"))
            .unwrap()
            .contains("voice_id=tide&"));
        assert!(stream_url(&c, Some("kokoro/zf_001")).is_err());
        assert!(stream_url(&c, Some("a&b=c")).is_err());
        c.voice = String::new();
        c.base_url = String::new();
        assert!(stream_url(&c, None)
            .unwrap()
            .starts_with("wss://api.deepseek.com/v1/tools/tts?voice_id=stella"));
    }

    #[test]
    fn pcm_is_little_endian_and_survives_odd_frame_splits() {
        let mut odd = None;
        let bytes: Vec<u8> = [1i16, -2, 300, i16::MIN]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let mut out = pcm_le(&bytes[..3], &mut odd);
        assert_eq!(odd, Some(bytes[2]));
        out.extend(pcm_le(&bytes[3..4], &mut odd));
        out.extend(pcm_le(&[], &mut odd));
        out.extend(pcm_le(&bytes[4..], &mut odd));
        assert_eq!(out, vec![1, -2, 300, i16::MIN]);
        assert_eq!(odd, None);
    }

    #[test]
    fn streams_text_in_order_and_receives_all_audio() {
        let (base, seen) = fake_server(Script::echo(7));
        let (stream, rx) = DeepseekStream::open_channel(&config(&base), None).unwrap();
        stream.send_text("能听到，");
        stream.send_text("很清楚。");
        stream.send_text("  ");
        stream.finish_text();
        let (audio, end) = collect(&rx);
        assert_eq!(end, Some(StreamEvent::Finished));
        assert_eq!(audio.len(), 8 * 7);
        assert!(audio.iter().all(|s| *s == 400));
        stream.join();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.authorization, "Bearer sk-test-not-real");
        assert_eq!(
            seen.query,
            "voice_id=stella&format=pcm&filter_markdown=true"
        );
        assert_eq!(seen.texts, vec!["能听到，", "很清楚。"]);
        assert_eq!(seen.finished_text, 1);
        assert_eq!(seen.connections, 1);
    }

    #[test]
    fn content_filter_and_other_codes_become_readable_errors() {
        let (base, _) = fake_server(Script {
            filter_word: Some("违禁"),
            ..Script::echo(5)
        });
        let (stream, rx) = DeepseekStream::open_channel(&config(&base), None).unwrap();
        stream.send_text("一段违禁的话。");
        stream.finish_text();
        let (audio, end) = collect(&rx);
        assert!(audio.is_empty());
        assert_eq!(
            end,
            Some(StreamEvent::Rejected(finish_error("content_filter")))
        );
        let (base, _) = fake_server(Script {
            finish_code: "service_error",
            ..Script::echo(5)
        });
        let (stream, rx) = DeepseekStream::open_channel(&config(&base), None).unwrap();
        stream.send_text("一段话。");
        stream.finish_text();
        let (audio, end) = collect(&rx);
        assert_eq!(audio.len(), 20);
        assert_eq!(
            end,
            Some(StreamEvent::Failed(finish_error("service_error")))
        );
        assert!(finish_error("content_filter").contains("内容审核"));
        assert!(finish_error("weird").contains("weird"));
    }

    #[test]
    fn rejected_key_is_reported_without_echoing_it() {
        let (base, _) = fake_server(Script {
            reject: true,
            ..Script::echo(1)
        });
        let (_stream, rx) = DeepseekStream::open_channel(&config(&base), None).unwrap();
        let (_, end) = collect(&rx);
        let Some(StreamEvent::Failed(error)) = end else {
            panic!("expected failure, got {end:?}");
        };
        assert!(error.contains("Key 无效"), "{error}");
        assert!(!error.contains("sk-test"));
    }

    #[test]
    fn cancel_closes_the_connection_immediately_without_further_events() {
        let (base, seen) = fake_server(Script::echo(3));
        let (stream, rx) = DeepseekStream::open_channel(&config(&base), None).unwrap();
        stream.send_text("第一句。");
        // 等第一块声音回来再取消
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(5)),
            Ok(StreamEvent::Audio(_))
        ));
        let started = Instant::now();
        stream.cancel();
        stream.send_text("不该发出去。");
        stream.join();
        assert!(started.elapsed() < Duration::from_millis(500));
        // 只剩已经在路上的声音，没有 Finished / Failed
        while let Ok(event) = rx.try_recv() {
            assert!(matches!(event, StreamEvent::Audio(_)), "{event:?}");
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while seen.lock().unwrap().closed_early == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let seen = seen.lock().unwrap();
        assert_eq!(seen.closed_early, 1);
        assert_eq!(seen.texts, vec!["第一句。"]);
    }

    #[test]
    fn missing_key_fails_before_connecting() {
        let mut c = config("http://127.0.0.1:9");
        c.api_key = " ".into();
        assert!(DeepseekStream::open(&c, None, |_| {})
            .err()
            .unwrap()
            .contains("Key"));
    }

    /// 真接口：`KOTONE_REAL_DEEPSEEK_KEY` 只从环境变量读，不打印。量第一块声音的延迟、
    /// 总时长，并核对字节序（小端解出来应当是平滑的语音波形）。
    #[test]
    #[ignore = "needs KOTONE_REAL_DEEPSEEK_KEY and network; costs a few cents"]
    fn real_deepseek_stream_latency_and_byte_order() {
        let key = std::env::var("KOTONE_REAL_DEEPSEEK_KEY").expect("KOTONE_REAL_DEEPSEEK_KEY");
        let mut c = config(crate::online::DEEPSEEK_ORIGIN);
        c.api_key = key;
        let sentences = [
            "好的，",
            "我来看看对面的出装。",
            "他们打野刚买了真眼，小龙那边要小心！",
        ];
        // 先连上（实际使用时连接在模型生成第一句的时间里建立），再从发第一句开始计时。
        let opened = Instant::now();
        let (stream, rx) = DeepseekStream::open_channel(&c, None).unwrap();
        std::thread::sleep(Duration::from_millis(1500));
        // 第一句以逗号结尾、不发 finish_text：服务端会不会等更多文字才出声？
        let started = Instant::now();
        stream.send_text(sentences[0]);
        let mut first_audio = None;
        let mut audio = Vec::new();
        let mut chunks = 0;
        let mut finish_sent = false;
        let mut sent_rest_at = None;
        let end = loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(StreamEvent::Audio(chunk)) => {
                    first_audio.get_or_insert_with(|| started.elapsed());
                    chunks += 1;
                    audio.extend(chunk);
                }
                Ok(other) => break other,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => panic!("stream ended without event"),
            }
            // 第一块声音回来以后（或 3 秒还没有），再发后两句并结束
            if !finish_sent && (first_audio.is_some() || started.elapsed() > Duration::from_secs(3))
            {
                sent_rest_at = Some(started.elapsed());
                stream.send_text(sentences[1]);
                stream.send_text(sentences[2]);
                stream.finish_text();
                finish_sent = true;
            }
        };
        let total = started.elapsed();
        stream.join();
        assert_eq!(end, StreamEvent::Finished, "{end:?}");
        let (le_smooth, be_smooth) = smoothness(&audio);
        let peak = audio.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        let chars: usize = sentences
            .iter()
            .map(|s| crate::segment::spoken_chars(s))
            .sum();
        println!(
            "real_deepseek_stream: opened {:?} ago, slept 1.5 s before t0; \
             first_audio={first_audio:?} after the first piece was sent alone without finish_text, \
             rest_sent_at={sent_rest_at:?}, finished={total:?}, chunks={chunks}, \
             audio={:.2}s for {chars} spoken chars, peak={peak}, \
             mean|d| little-endian={le_smooth:.0} vs byte-swapped={be_smooth:.0}",
            opened.elapsed(),
            audio.len() as f64 / SAMPLE_RATE as f64
        );
        assert!(audio.len() > SAMPLE_RATE as usize, "less than 1 s of audio");
        assert!(
            le_smooth * 3.0 < be_smooth,
            "little-endian samples are not smoother than byte-swapped ones"
        );
        if let Ok(dir) = std::env::var("KOTONE_REAL_TTS_WAV_DIR") {
            let path = std::path::Path::new(&dir).join("deepseek-stream.wav");
            std::fs::write(&path, wav_bytes(&audio)).unwrap();
            println!("real_deepseek_stream: wrote {}", path.display());
        }
    }

    /// 真接口：服务端什么时候开始出声？每种情况一个新会话（先连好再计时）。
    #[test]
    #[ignore = "needs KOTONE_REAL_DEEPSEEK_KEY and network; costs a few cents"]
    fn real_deepseek_stream_when_does_audio_start() {
        let key = std::env::var("KOTONE_REAL_DEEPSEEK_KEY").expect("KOTONE_REAL_DEEPSEEK_KEY");
        let mut c = config(crate::online::DEEPSEEK_ORIGIN);
        c.api_key = key;
        let cases: &[(&str, &[&str], bool)] = &[
            ("comma piece, no finish", &["好的，"], false),
            ("one sentence, no finish", &["我来看看对面的出装。"], false),
            (
                "three sentences, no finish",
                &[
                    "好的，",
                    "我来看看对面的出装。",
                    "他们打野刚买了真眼，小龙那边要小心！",
                ],
                false,
            ),
            ("comma piece + finish", &["好的，"], true),
            ("one sentence + finish", &["我来看看对面的出装。"], true),
        ];
        for (name, pieces, finish) in cases {
            let (stream, rx) = DeepseekStream::open_channel(&c, None).unwrap();
            std::thread::sleep(Duration::from_millis(1200));
            let started = Instant::now();
            for piece in *pieces {
                stream.send_text(piece);
            }
            if *finish {
                stream.finish_text();
            }
            let mut first = None;
            let mut samples = 0usize;
            let mut finish_at = None;
            let end = loop {
                match rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(StreamEvent::Audio(chunk)) => {
                        first.get_or_insert_with(|| started.elapsed());
                        samples += chunk.len();
                    }
                    Ok(other) => break other,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => panic!("{name}: no end event"),
                }
                if finish_at.is_none() && !*finish && started.elapsed() > Duration::from_secs(4) {
                    finish_at = Some(started.elapsed());
                    stream.finish_text();
                }
            };
            println!(
                "probe [{name}]: first_audio={first:?}, late finish_text at {finish_at:?}, \
                 ended={:?} {end:?}, audio={:.2}s",
                started.elapsed(),
                samples as f64 / SAMPLE_RATE as f64
            );
            stream.join();
        }
    }

    /// 相邻样本差的平均绝对值：语音是低频为主的平滑波形，字节序对时这个值远小于字节序反了。
    pub(crate) fn smoothness(samples: &[i16]) -> (f64, f64) {
        let diff = |values: &[i32]| {
            values
                .windows(2)
                .map(|w| (w[1] - w[0]).abs() as f64)
                .sum::<f64>()
                / values.len().max(2) as f64
        };
        let le: Vec<i32> = samples.iter().map(|s| *s as i32).collect();
        let be: Vec<i32> = samples.iter().map(|s| s.swap_bytes() as i32).collect();
        (diff(&le), diff(&be))
    }

    fn wav_bytes(samples: &[i16]) -> Vec<u8> {
        let data_len = (samples.len() * 2) as u32;
        let mut out = Vec::with_capacity(44 + data_len as usize);
        out.extend(b"RIFF");
        out.extend((36 + data_len).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16u32.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(SAMPLE_RATE.to_le_bytes());
        out.extend((SAMPLE_RATE * 2).to_le_bytes());
        out.extend(2u16.to_le_bytes());
        out.extend(16u16.to_le_bytes());
        out.extend(b"data");
        out.extend(data_len.to_le_bytes());
        out.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
        out
    }
}
