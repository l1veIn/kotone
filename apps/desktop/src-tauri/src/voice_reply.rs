//! 语音回合边生成边念 `speak`（ADR 029 第六、八节）：只有 `speak` 里的字会念出来，
//! 模型一边写 `speak` 的参数，这里一边取出字交给 [`SpeechStream`]，不等整句写完。
//!
//! - 参数是一段一段流过来的 JSON（[`AgentEvent::ToolCallChunk`]），每次从拼到现在的整段里取出
//!   `text`（[`kotone_agent::speak::text_so_far`]），把多出来的字喂进去。
//! - 这一轮开始前就把第一路准备好（DeepSeek 在后台先连好，握手藏在模型想第一句的时间里），
//!   但不占播报：第一个字到了才排队拿播报锁（[`crate::assistant::hold_output`]）。前面还在播填充话、
//!   提醒时只合成不出声，轮到它了再放开；麦克风开着时也先不出声。
//! - `speak` 真正执行时（[`VoiceTurn::take`]）把剩下的字补上、收尾，等念完（或被打断）才返回，
//!   工具结果里写“说完了”或“只说到：……”——不用回头改对话记录。
//! - 打断（呼叫键、唤醒词、开始录音、“停下”）：[`interrupt`] 立刻停掉，等输出设备关掉才返回。
//!
//! 照 LiveKit Agents（Apache-2.0）`voice/agent_activity.py`：边生成边合成、按顺序播，打断时只算已经播出去的字。
//!
//! [`AgentEvent::ToolCallChunk`]: kotone_ai::agent::AgentEvent::ToolCallChunk

use std::collections::HashMap;
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
/// 念一句最多等多久（DeepSeek 一路会话在发完字 20 秒后自己结束）。
const FINISH_WAIT: Duration = Duration::from_secs(60);

/// 已经开口（或排着队要开口）、还没结束的：打断时一起停。
static ACTIVE: Mutex<Vec<Arc<Reply>>> = Mutex::new(Vec::new());

/// 一次 `speak`：一路边生成边念。
pub struct Reply {
    app: AppHandle,
    stream: SpeechStream,
    /// 已经喂进去的字。
    fed: Mutex<String>,
    interrupted: AtomicBool,
    /// 模型撤回了这一步（内容审核、空回答重发）。
    discarded: AtomicBool,
    done: tokio::sync::watch::Sender<bool>,
}

/// 一次 `speak` 念完以后的情况。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spoken {
    /// 都念出来了。
    Done,
    /// 被打断了：玩家听到的那几句（按整句算）。
    Interrupted(String),
    Failed(String),
}

impl Reply {
    fn feed(&self, text: &str) {
        let mut fed = self.fed.lock().unwrap();
        if let Some(rest) = text.strip_prefix(fed.as_str()) {
            if !rest.is_empty() {
                self.stream.push_text(rest);
                fed.push_str(rest);
            }
        }
    }

    /// 停下；返回是不是真的打断了什么（已经念完的不算）。阻塞到输出设备关掉（最多 2 秒）。
    fn interrupt(&self) -> bool {
        if self.discarded.load(Ordering::SeqCst) || self.stream.is_done() {
            return false;
        }
        self.interrupted.store(true, Ordering::SeqCst);
        if let Err(e) = self.stream.cancel() {
            log::log(&format!("voice reply: {e}"));
        }
        true
    }

    /// 补上最后的字、收尾，等它念完（或被打断）。
    pub async fn finish(&self, text: &str) -> Spoken {
        self.feed(text);
        let result = tokio::time::timeout(FINISH_WAIT, self.stream.finish())
            .await
            .unwrap_or_else(|_| {
                let _ = self.stream.cancel();
                Err("念得太久，停下了".into())
            });
        let mut done = self.done.subscribe();
        let _ = done.wait_for(|d| *d).await;
        if self.interrupted.load(Ordering::SeqCst) {
            return Spoken::Interrupted(self.stream.spoken_text());
        }
        match result {
            Ok(()) => Spoken::Done,
            // 念出来一部分就出错（例如有一句没过内容审核）：照被打断算，玩家听到了前面的
            Err(_) if !self.stream.spoken_text().trim().is_empty() => {
                Spoken::Interrupted(self.stream.spoken_text())
            }
            Err(e) => Spoken::Failed(e),
        }
    }
}

/// 一次 `speak` 调用（这一步里第 index 个工具调用）的参数和念它的那一路。
#[derive(Default)]
struct Slot {
    raw: String,
    name: String,
    id: String,
    reply: Option<Arc<Reply>>,
}

/// 一个语音回合（游戏内入口的一轮）。
pub struct VoiceTurn {
    app: AppHandle,
    conversation: String,
    owner: Option<Owner>,
    bubble: bool,
    /// 弹幕也显示（关了气泡，或开了“回答同时用弹幕”）。
    danmaku: bool,
    /// 准备好的下一路（还没开始念，不占播报）。
    next: Mutex<Option<SpeechStream>>,
    /// 这一步里的工具调用（按序号）。
    slots: Mutex<Vec<Slot>>,
    /// 已经开始念、等 `speak` 执行的那几路：调用 id → 那一路。
    started: Mutex<HashMap<String, Arc<Reply>>>,
}

impl VoiceTurn {
    /// 回复方式里有语音、流式语音准备得起来才边生成边念；否则 None（`speak` 执行时整段念）。
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
            danmaku: wanted.contains(&DeliveryKind::Danmaku) && (!bubble || also_danmaku),
            next: Mutex::new(None),
            slots: Mutex::default(),
            started: Mutex::default(),
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
        // 合成费用记到这一轮；轮中插了话就记到插话那一轮。
        options.owner = kotone_core::usage::current_turn()
            .filter(|t| t.conversation == self.conversation)
            .or_else(|| self.owner.clone());
        SpeechStream::start(engine, options)
    }

    /// 新的一步（又调用了一次模型）：工具调用从 0 重新编号。
    pub fn on_step(&self) {
        self.slots.lock().unwrap().clear();
    }

    /// 工具参数的一段。返回这段是不是让她开口了（这一轮第一次有字）。
    pub fn on_args(&self, index: usize, id: &str, name: &str, chunk: &str) -> bool {
        let mut slots = self.slots.lock().unwrap();
        while slots.len() <= index {
            slots.push(Slot::default());
        }
        let slot = &mut slots[index];
        slot.raw.push_str(chunk);
        slot.name = name.to_string();
        slot.id = id.to_string();
        if slot.name != kotone_agent::catalog::TOOL_SPEAK {
            return false;
        }
        let text = kotone_agent::speak::text_so_far(&slot.raw);
        if let Some(reply) = &slot.reply {
            reply.feed(&text);
            return false;
        }
        if text.trim().is_empty() {
            return false;
        }
        let prepared = self.next.lock().unwrap().take();
        let stream = match prepared.map(Ok).unwrap_or_else(|| self.prepare()) {
            Ok(stream) => stream,
            Err(e) => {
                log::log(&format!("voice reply: 不能边生成边念，整段念：{e}"));
                return false;
            }
        };
        let reply = Arc::new(Reply {
            app: self.app.clone(),
            stream,
            fed: Mutex::default(),
            interrupted: AtomicBool::new(false),
            discarded: AtomicBool::new(false),
            done: tokio::sync::watch::channel(false).0,
        });
        reply.feed(&text);
        slot.reply = Some(reply.clone());
        ACTIVE.lock().unwrap().push(reply.clone());
        crate::voice_timing::tts_started();
        tauri::async_runtime::spawn(play(reply, self.bubble));
        true
    }

    /// 模型发起了完整的工具调用：开始念了的那一路记到它的调用 id 下（`speak` 执行时取）。
    pub fn on_call(&self, id: &str) {
        let slots = self.slots.lock().unwrap();
        if let Some(reply) = slots
            .iter()
            .find(|s| s.id == id)
            .and_then(|s| s.reply.clone())
        {
            self.started.lock().unwrap().insert(id.to_string(), reply);
        }
    }

    /// 模型撤回了这一步（内容审核没过、空回答重发）：念到一半的马上停。
    pub fn on_discard(&self) {
        let replies: Vec<Arc<Reply>> = self
            .slots
            .lock()
            .unwrap()
            .drain(..)
            .filter_map(|s| s.reply)
            .collect();
        if replies.is_empty() {
            return;
        }
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            for reply in replies {
                reply.discarded.store(true, Ordering::SeqCst);
                if let Err(e) = reply.stream.cancel() {
                    log::log(&format!("voice reply: {e}"));
                }
            }
            crate::bubble::stop_speaking(&app);
        });
    }

    /// `speak` 执行了：取出已经开始念的那一路（没有 = 流式用不了或没来得及，整段念）。
    /// 弹幕这时显示整句。
    pub fn take(&self, call_id: &str, text: &str) -> Option<Arc<Reply>> {
        if self.danmaku {
            crate::assistant::show_danmaku(&self.app, text);
        }
        self.started.lock().unwrap().remove(call_id)
    }

    /// 这一轮跑完了：没用上的那路关掉（DeepSeek 按发出去的字计费，空连接不花钱）；
    /// 开始念了却没执行的 `speak`（例如被插话跳过）让它念完。
    pub fn close(&self) {
        drop(self.next.lock().unwrap().take());
        let left: Vec<Arc<Reply>> = self
            .started
            .lock()
            .unwrap()
            .drain()
            .map(|(_, r)| r)
            .chain(self.slots.lock().unwrap().drain(..).filter_map(|s| s.reply))
            .collect();
        for reply in left {
            tauri::async_runtime::spawn(async move {
                let _ = reply.stream.finish().await;
            });
        }
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
                crate::voice_timing::sound_at("speak", epoch_ms(at));
            }
        }
        let quiet =
            reply.interrupted.load(Ordering::SeqCst) || reply.discarded.load(Ordering::SeqCst);
        if bubble && !quiet {
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
    let quiet = reply.interrupted.load(Ordering::SeqCst) || reply.discarded.load(Ordering::SeqCst);
    if let Some(revision) = revision.filter(|_| !quiet) {
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

/// 有没有正在念（或排队要念）的。
pub fn active() -> bool {
    !ACTIVE.lock().unwrap().is_empty()
}

/// 打断：正在边生成边念的都停下（等输出设备关掉才返回，最多约 2 秒）。返回是不是打断了什么。
/// 阻塞，不要在异步线程里直接调用太久。
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

/// 等正在念的都念完（追问前：前面那句念完再问），最多 `max`。
pub async fn wait_idle(max: Duration) {
    let started = Instant::now();
    while active() && started.elapsed() < max {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// `speak` 的工具结果：说完了，或玩家打断了、只听到前面几句（ADR 029 第八节：
/// 不回头改对话记录，模型从这里知道玩家听到了什么）。
pub fn spoken_result(spoken: &Spoken) -> Result<serde_json::Value, String> {
    match spoken {
        Spoken::Done => Ok(serde_json::json!({"result": "说完了"})),
        Spoken::Interrupted(heard) if heard.trim().is_empty() => {
            Ok(serde_json::json!({"result": "玩家打断了，一句也没听到"}))
        }
        Spoken::Interrupted(heard) => Ok(serde_json::json!({
            "result": format!("玩家打断了，只说到：{}", heard.trim())
        })),
        Spoken::Failed(e) => Err(format!("没能念出来：{e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_result_says_what_the_player_heard() {
        assert_eq!(spoken_result(&Spoken::Done).unwrap()["result"], "说完了");
        assert_eq!(
            spoken_result(&Spoken::Interrupted("先出无尽。".into())).unwrap()["result"],
            "玩家打断了，只说到：先出无尽。"
        );
        assert_eq!(
            spoken_result(&Spoken::Interrupted(" ".into())).unwrap()["result"],
            "玩家打断了，一句也没听到"
        );
        assert!(spoken_result(&Spoken::Failed("断了".into())).is_err());
    }
}
