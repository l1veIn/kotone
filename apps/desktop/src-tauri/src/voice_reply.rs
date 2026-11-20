//! 语音回合边生成边念（ADR 029 §六，第三期）：模型写一段就念一段，不等整段回答写完再合成。
//!
//! - 每次调用模型（一步）一路 [`SpeechStream`]：正文增量（`MessageChunk`）进来就喂，推理文字永不喂；
//!   这一步写完（`Appended`）就收尾，念完才算结束。工具前的那句（“我查一下”）也这样念出来。
//! - 下一路提前准备好（DeepSeek 在后台先连好，握手藏在模型想第一句的时间里），但不占播报：
//!   第一段正文到了才排队拿播报锁（[`crate::assistant::hold_output`]）。前面还在播填充话、提醒时
//!   只合成不出声，轮到它了再放开；麦克风开着时也先不出声（和提醒一样，不把自己的话录进去）。
//! - 气泡跟着念到的句子更新；第一次出声写进计时（[`crate::voice_timing::sound_at`]）。
//! - 打断（呼叫键、唤醒词、开始录音、“停下”）：[`interrupt`] 立刻停掉正在念的，等输出设备关掉
//!   才返回；对话里这条回答改成玩家实际听到的部分（[`crate::agent::truncate_spoken`]）。
//! - 模型用了 `reply.say`、或流式语音用不了：这一轮剩下的文字照旧整段念（`agent` 里的老路）。
//!
//! 照 LiveKit Agents（Apache-2.0）`voice/agent_activity.py`：回答边生成边合成、按顺序播，打断时
//! 只把已经播出去的文字写回对话（`_truncate_message`，消息标 `interrupted`）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kotone_core::log;
use kotone_core::reminders::DeliveryKind;
use kotone_core::usage::Owner;
use kotone_tts::SpeechStream;
use tauri::{AppHandle, Manager};

/// 麦克风开着时最多等这么久再出声（同提醒）；还开着就不念了。
const CAPTURE_WAIT: Duration = Duration::from_secs(30);
/// 多久看一次念到哪了（气泡、计时）。
const POLL: Duration = Duration::from_millis(100);

/// 已经开口（或排着队要开口）、还没结束的回答：打断时一起停。
static ACTIVE: Mutex<Vec<Arc<Reply>>> = Mutex::new(Vec::new());

/// 一步（一次模型调用）的正文：一路边生成边念。
struct Reply {
    app: AppHandle,
    stream: SpeechStream,
    conversation: String,
    /// 对话里这条消息的时间（这一步写完才知道）。
    message: Mutex<Option<u64>>,
    /// 这一步后面跟着工具调用（计时里记成 `preamble`）。
    tools: AtomicBool,
    /// 被打断时玩家已经听到的部分（等知道是哪条消息再改对话记录）。
    interrupted: Mutex<Option<String>>,
    /// 模型撤回了这一步（内容审核、空回答重发）：不进对话，不用改记录。
    discarded: AtomicBool,
    done: tokio::sync::watch::Sender<bool>,
}

/// 一路念完以后的情况。
#[derive(Debug, Clone)]
pub struct Spoken {
    /// 对话里那条消息的时间。
    pub message: Option<u64>,
    /// 模型写的全文。
    pub text: String,
    /// 完整念出来的句子。
    pub spoken: String,
    pub interrupted: bool,
    pub error: Option<String>,
}

impl Reply {
    fn was_interrupted(&self) -> bool {
        self.interrupted.lock().unwrap().is_some()
    }

    /// 停下；返回是不是真的打断了什么（已经念完的不算）。阻塞到输出设备关掉（最多 2 秒）。
    fn interrupt(&self) -> bool {
        if self.discarded.load(Ordering::SeqCst) || self.stream.is_done() {
            return false;
        }
        if let Err(e) = self.stream.cancel() {
            log::log(&format!("voice reply: {e}"));
        }
        let spoken = self.stream.spoken_text();
        // 和 `bind` 交叉加锁：两边至少有一边看得到对方，记录一定会改（改两次也一样）。
        *self.interrupted.lock().unwrap() = Some(spoken.clone());
        let message = *self.message.lock().unwrap();
        if let Some(at) = message {
            // 这一步写完了、每句都念完了（只剩设备里最后一点尾音）：记录不用改
            if spoken.trim() != self.stream.full_text().trim() {
                crate::agent::truncate_spoken(&self.app, &self.conversation, at, &spoken);
            }
        }
        true
    }

    /// 这一步写完了：记下是哪条消息；之前已经被打断就把记录改成听到的部分。
    fn bind(&self, at_ms: u64) {
        *self.message.lock().unwrap() = Some(at_ms);
        let pending = self.interrupted.lock().unwrap().clone();
        if let Some(spoken) = pending {
            crate::agent::truncate_spoken(&self.app, &self.conversation, at_ms, &spoken);
        }
    }

    async fn wait_done(&self) {
        let mut done = self.done.subscribe();
        let _ = done.wait_for(|d| *d).await;
    }
}

/// 一个语音回合（游戏内入口的一轮）：一步一路边生成边念。
pub struct VoiceTurn {
    app: AppHandle,
    conversation: String,
    owner: Option<Owner>,
    bubble: bool,
    danmaku: bool,
    /// 准备好的下一路（还没开始念，不占播报）。
    next: Mutex<Option<SpeechStream>>,
    /// 这一步正在写的那路。
    current: Mutex<Option<Arc<Reply>>>,
    replies: Mutex<Vec<Arc<Reply>>>,
    /// 这一轮剩下的文字不再边生成边念（用了 `reply.say`、流式语音用不了）。
    muted: AtomicBool,
    /// 已经说了“交给我”、后台在做：这一轮最后的话只写进聊天，不念（也不走整段念）。
    handed_off: AtomicBool,
}

impl VoiceTurn {
    /// 回复方式里有语音、流式语音准备得起来才边生成边念；否则 None（整段念）。
    pub fn new(app: &AppHandle, conversation: &str, owner: Option<Owner>) -> Option<Arc<Self>> {
        let (wanted, also_danmaku) = {
            let settings = app.state::<crate::SharedState>();
            let settings = settings.settings.read().unwrap();
            (
                settings.assistant.default_delivery.clone(),
                settings.assistant.bubble.also_danmaku,
            )
        };
        if !wanted.contains(&DeliveryKind::Voice) {
            return None;
        }
        let bubble = crate::bubble::enabled(app);
        let turn = Self {
            app: app.clone(),
            conversation: conversation.to_string(),
            owner,
            bubble,
            // 弹幕照旧显示全文（关了气泡，或打开了“回答同时用弹幕”）
            danmaku: wanted.contains(&DeliveryKind::Danmaku) && (!bubble || also_danmaku),
            next: Mutex::new(None),
            current: Mutex::new(None),
            replies: Mutex::new(Vec::new()),
            muted: AtomicBool::new(false),
            handed_off: AtomicBool::new(false),
        };
        match turn.prepare() {
            Ok(stream) => *turn.next.lock().unwrap() = Some(stream),
            Err(e) => {
                log::log(&format!("voice reply: 不能边生成边念，整段念：{e}"));
                return None;
            }
        }
        Some(Arc::new(turn))
    }

    fn prepare(&self) -> Result<SpeechStream, String> {
        let (engine, mut options) = crate::assistant::stream_engine(&self.app)?;
        options.held = true;
        // 合成费用记到这一轮；轮中插了话就记到插话那一轮（同整段念时“最后一句玩家的话”）。
        options.owner = kotone_core::usage::current_turn()
            .filter(|t| t.conversation == self.conversation)
            .or_else(|| self.owner.clone());
        SpeechStream::start(engine, options)
    }

    /// 这一轮剩下的文字走整段念的老路（`reply.say` 已经说过、流式语音用不了）。
    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::SeqCst)
    }

    /// 用了 `reply.say`：她已经开口说过，这一轮后面的文字不再念（同整段念时的规矩）。
    pub fn mute(&self) {
        self.muted.store(true, Ordering::SeqCst);
    }

    /// 说过“交给我”、后台在做：这一轮后面的话不再念。
    pub fn hand_off(&self) {
        self.handed_off.store(true, Ordering::SeqCst);
        self.mute();
    }

    pub fn handed_off(&self) -> bool {
        self.handed_off.load(Ordering::SeqCst)
    }

    /// 这一轮念过（或在念）东西吗。
    pub fn has_replies(&self) -> bool {
        !self.replies.lock().unwrap().is_empty()
    }

    /// 模型正文的一段。返回这段是不是让她开口了（这一步第一次有字）。
    pub fn on_chunk(&self, text: &str) -> bool {
        if self.muted() {
            return false;
        }
        let mut current = self.current.lock().unwrap();
        if let Some(reply) = current.as_ref() {
            reply.stream.push_text(text);
            return false;
        }
        if text.trim().is_empty() {
            return false;
        }
        let prepared = self.next.lock().unwrap().take();
        let stream = match prepared.map(Ok).unwrap_or_else(|| self.prepare()) {
            Ok(stream) => stream,
            Err(e) => {
                log::log(&format!(
                    "voice reply: 不能边生成边念，这一轮剩下的整段念：{e}"
                ));
                self.mute();
                return false;
            }
        };
        stream.push_text(text);
        let reply = Arc::new(Reply {
            app: self.app.clone(),
            stream,
            conversation: self.conversation.clone(),
            message: Mutex::new(None),
            tools: AtomicBool::new(false),
            interrupted: Mutex::new(None),
            discarded: AtomicBool::new(false),
            done: tokio::sync::watch::channel(false).0,
        });
        *current = Some(reply.clone());
        self.replies.lock().unwrap().push(reply.clone());
        ACTIVE.lock().unwrap().push(reply.clone());
        crate::voice_timing::tts_started();
        tauri::async_runtime::spawn(play(reply, self.bubble));
        true
    }

    /// 这一步写完了（`Appended`）：收尾（念完为止），弹幕显示全文；后面还要调用模型就把下一路准备好。
    pub fn on_step_end(&self, text: &str, tools: bool, at_ms: u64) {
        let reply = self.current.lock().unwrap().take();
        if let Some(reply) = reply {
            reply.tools.store(tools, Ordering::SeqCst);
            reply.bind(at_ms);
            tauri::async_runtime::spawn(async move {
                let _ = reply.stream.finish().await;
            });
            if self.danmaku && !text.trim().is_empty() {
                crate::assistant::show_danmaku(&self.app, text.trim());
            }
        }
        if tools && !self.muted() {
            let mut next = self.next.lock().unwrap();
            if next.is_none() {
                match self.prepare() {
                    Ok(stream) => *next = Some(stream),
                    Err(e) => log::log(&format!("voice reply: 下一步没能先准备好：{e}")),
                }
            }
        }
    }

    /// 模型撤回了这一步（内容审核没过、空回答重发）：念到一半的马上停。
    pub fn on_discard(&self) {
        let Some(reply) = self.current.lock().unwrap().take() else {
            return;
        };
        reply.discarded.store(true, Ordering::SeqCst);
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(e) = reply.stream.cancel() {
                log::log(&format!("voice reply: {e}"));
            }
            crate::bubble::stop_speaking(&app);
        });
    }

    /// 这一轮跑完了：没用上的那路关掉（DeepSeek 按发出去的字计费，空连接不花钱）；还没收尾的那步收尾。
    pub fn close(&self) {
        drop(self.next.lock().unwrap().take());
        if let Some(reply) = self.current.lock().unwrap().take() {
            tauri::async_runtime::spawn(async move {
                let _ = reply.stream.finish().await;
            });
        }
    }

    /// 等这一轮念的都结束（念完、出错或被打断），按顺序返回每一路的情况。先 [`close`](Self::close)。
    pub async fn wait_all(&self) -> Vec<Spoken> {
        let replies = self.replies.lock().unwrap().clone();
        let mut out = Vec::new();
        for reply in replies {
            if reply.discarded.load(Ordering::SeqCst) {
                continue;
            }
            let result = reply.stream.finish().await;
            reply.wait_done().await;
            let interrupted = reply.was_interrupted();
            out.push(Spoken {
                message: *reply.message.lock().unwrap(),
                text: reply.stream.full_text(),
                spoken: reply.stream.spoken_text(),
                interrupted,
                error: result.err().filter(|_| !interrupted),
            });
        }
        out
    }
}

/// 一路的播放：排队拿播报、等麦克风关、放开出声；念的时候更新气泡、记计时；念完收尾。
async fn play(reply: Arc<Reply>, bubble: bool) {
    let app = reply.app.clone();
    // 1. 前面的填充话、提醒播完才轮到这句（这期间照常合成）
    let guard = tokio::select! {
        guard = crate::assistant::hold_output(&app) => guard,
        _ = until_done(&reply) => None,
    };
    // 2. 玩家正开着麦：先不出声（免得把自己的话录进去）；一直开着就不念了
    let waited = Instant::now();
    while crate::assistant::capturing(&app) && !reply.stream.is_done() {
        if waited.elapsed() >= CAPTURE_WAIT {
            log::log("voice reply: 麦克风一直开着，这句不念了");
            reply.interrupt();
            break;
        }
        tokio::time::sleep(POLL).await;
    }
    reply.stream.release_playback();
    // 3. 念：气泡跟着念到的句子换，第一次出声记计时
    let mut shown = String::new();
    let mut revision = None;
    let mut timed = false;
    loop {
        let done = reply.stream.is_done();
        if !timed {
            if let Some(at) = reply.stream.first_audio_at() {
                timed = true;
                let kind = if reply.tools.load(Ordering::SeqCst) {
                    "preamble"
                } else {
                    "answer"
                };
                crate::voice_timing::sound_at(kind, epoch_ms(at));
            }
        }
        if bubble && !reply.was_interrupted() && !reply.discarded.load(Ordering::SeqCst) {
            let text = reply.stream.started_text();
            let text = text.trim();
            if !text.is_empty() && text != shown {
                shown = text.to_string();
                revision = crate::bubble::speaking(&app, &shown).or(revision);
            }
        }
        if done {
            break;
        }
        tokio::time::sleep(POLL).await;
    }
    drop(guard);
    // 4. 念完：气泡按字数停一会儿再收（被打断时已经收起了）
    if let Some(revision) =
        revision.filter(|_| !reply.was_interrupted() && !reply.discarded.load(Ordering::SeqCst))
    {
        crate::bubble::spoken(&app, revision);
    }
    ACTIVE.lock().unwrap().retain(|r| !Arc::ptr_eq(r, &reply));
    reply.done.send_replace(true);
}

async fn until_done(reply: &Reply) {
    while !reply.stream.is_done() {
        tokio::time::sleep(POLL).await;
    }
}

/// `Instant` → 毫秒时间戳（计时日志用）。
fn epoch_ms(at: Instant) -> u64 {
    kotone_core::reminders::now_ms()
        .saturating_sub(Instant::now().saturating_duration_since(at).as_millis() as u64)
}

/// 有没有正在念（或排队要念）的回答。
pub fn active() -> bool {
    !ACTIVE.lock().unwrap().is_empty()
}

/// 打断：正在边生成边念的都停下（等输出设备关掉才返回，最多约 2 秒），对话里只留听到的部分。
/// 返回是不是打断了什么。阻塞，不要在异步线程里直接调用太久。
pub fn interrupt() -> bool {
    let active: Vec<Arc<Reply>> = ACTIVE.lock().unwrap().clone();
    let mut any = false;
    for reply in active {
        any |= reply.interrupt();
    }
    if any {
        log::log("voice reply: interrupted");
    }
    any
}

/// 等正在念的回答都念完（追问前：工具前那句念完再问），最多 `max`。
pub async fn wait_idle(max: Duration) {
    let started = Instant::now();
    while active() && started.elapsed() < max {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
