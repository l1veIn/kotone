//! 边生成边念（ADR 029 第六节）：模型的文字一段段进来，分句（[`crate::segment`]），
//! 一句一句合成，收到声音就播；打断时立刻停，只把已经完整念出来的句子记进对话。
//!
//! 接进语音回合的调用顺序：
//! 1. 模型开始回答前 [`SpeechStream::start`]（DeepSeek 在后台先连好一路，握手藏在模型
//!    生成第一句的时间里）；
//! 2. 每段文字 [`push_text`](SpeechStream::push_text)（不阻塞）；
//! 3. 回答写完 [`finish`](SpeechStream::finish)`.await`（或 `finish_blocking`）：念完才返回；
//! 4. 玩家打断：[`cancel`](SpeechStream::cancel)（阻塞到输出设备关掉，最多 2 秒，之后才能开麦），
//!    再用 [`spoken_text`](SpeechStream::spoken_text) 记进对话；
//! 5. 计时：[`first_audio_at`](SpeechStream::first_audio_at) 是第一句开始出声的时刻。
//!
//! 前面还有别的话在播（填充话、提醒）时，用 [`StreamOptions::held`] 先只合成、不出声，轮到它了再
//! [`release_playback`](SpeechStream::release_playback)。[`StreamOptions::owner`] 让每句合成的费用
//! 记到这一轮（合成前按这句的文字认领，`kotone_core::usage::claim_speech`）。
//!
//! 合成方式：
//! - **DeepSeek**：实测（2026-11-20，`stream::tests::real_deepseek_stream_when_does_audio_start`）
//!   流式接口在收到 `finish_text` 之前不出声（三整句发过去等 4 秒也没有声音），收到后约 0.9 秒
//!   出第一块、很快收完。所以**每句一路会话**（发完这句立刻 `finish_text`），并且总是提前连好
//!   下一路；最多同时合成 [`MAX_IN_FLIGHT`] 句。每句的声音是分开的，句子边界是准的。
//! - **Kokoro / OpenAI 兼容 / 火山**：逐句 `SpeechOutput::synthesize`，排队播放
//!   （播这一句时合成下一句）。
//! - **系统语音**拿不到声音数据：逐句 `speak`。
//!
//! 参考：LiveKit Agents（Apache-2.0）`voice/agent_activity.py` 与 Pipecat（BSD-2）
//! `services/tts_service.py`：句子进合成队列、音频按顺序播放、打断时只保留玩家实际听到的文字。

use std::{
    collections::VecDeque,
    sync::{mpsc, Arc, Condvar, Mutex},
    thread::JoinHandle,
    time::{Duration, Instant},
};

use kotone_core::speech::{SpeechOutput, SpeechRequest};
use tokio::sync::watch;

use crate::online::OnlineConfig;
use crate::segment::{Segmenter, Sentence};
use crate::stream::{DeepseekStream, StreamEvent, SAMPLE_RATE};
use crate::stream_player::{AudioDevice, DefaultOutputDevice, PcmQueue, PlayEnd};
use crate::CANCELLED;

/// 同时在合成的句子最多这么多（DeepSeek 每个账号最多 50 路并发；多合成的句子被打断就白花钱）。
pub const MAX_IN_FLIGHT: usize = 2;
/// 已经合成好、还没播的声音超过这么多秒就先不合成下一句。
const LOOKAHEAD: Duration = Duration::from_secs(8);
/// 预先连好的会话放这么久就换一路（服务端 60 秒没有文字会结束会话）。
const SPARE_MAX_AGE: Duration = Duration::from_secs(45);
/// `cancel` 最多等输出设备关掉这么久。
const CANCEL_TIMEOUT: Duration = Duration::from_secs(2);

/// 用哪种方式合成。
pub enum StreamEngine {
    /// DeepSeek 流式 WebSocket（连接设置与整段合成相同，`voice` 可带 `deepseek/` 前缀）。
    Deepseek(OnlineConfig),
    /// 能拿到声音数据的引擎（Kokoro、OpenAI 兼容、火山）：逐句合成、排队播放。
    Synthesize(Arc<dyn SpeechOutput>),
    /// 系统语音：逐句 `speak`。
    Speak(Arc<dyn SpeechOutput>),
}

impl StreamEngine {
    fn label(&self) -> &'static str {
        match self {
            Self::Deepseek(_) => "deepseek",
            Self::Synthesize(_) => "synthesize",
            Self::Speak(_) => "speak",
        }
    }
}

/// 声音、音量（0–100）、语速（-10..=10，DeepSeek 没有语速参数）。
#[derive(Debug, Clone)]
pub struct StreamOptions {
    pub voice_id: Option<String>,
    pub volume: u16,
    pub rate: i32,
    /// 先只合成、不出声，等 [`SpeechStream::release_playback`]（前面还有别的话在播）。
    pub held: bool,
    /// 每句合成的费用记到哪一轮（合成前按这句的文字认领）。
    pub owner: Option<kotone_core::usage::Owner>,
}

impl Default for StreamOptions {
    fn default() -> Self {
        Self {
            voice_id: None,
            volume: 80,
            rate: 0,
            held: false,
            owner: None,
        }
    }
}

struct Entry {
    sentence: Sentence,
    /// 这句声音在队列里结束的位置（播到这里就算念完了）。
    end: Option<u64>,
    /// 系统语音：`speak` 正常返回就算念完了。
    spoken: bool,
    /// 系统语音：开始 `speak` 了（气泡显示念到哪句用）。
    started: bool,
    failed: bool,
}

#[derive(Default)]
struct State {
    entries: Vec<Entry>,
    input_closed: bool,
    cancelled: bool,
    error: Option<String>,
    result: Option<Result<(), String>>,
    first_spoken: Option<Instant>,
}

struct Shared {
    queue: Arc<PcmQueue>,
    state: Mutex<State>,
    done: Condvar,
    done_tx: watch::Sender<bool>,
    device_open: Mutex<bool>,
    device_closed: Condvar,
    /// 还不能出声（[`StreamOptions::held`]）。
    held: Mutex<bool>,
    released: Condvar,
    started: Instant,
}

impl Shared {
    fn fail(&self, error: String) {
        let mut state = self.state.lock().unwrap();
        if state.error.is_none() && !state.cancelled {
            state.error = Some(error);
        }
    }

    /// 等到可以出声；被停了返回 false。
    fn wait_released(&self) -> bool {
        let mut held = self.held.lock().unwrap();
        while *held {
            if self.queue.is_stopped() {
                return false;
            }
            held = self
                .released
                .wait_timeout(held, Duration::from_millis(100))
                .unwrap()
                .0;
        }
        !self.queue.is_stopped()
    }
}

enum Msg {
    Sentence(usize),
    InputDone,
    Cancel,
    Deepseek(u64, StreamEvent),
    Synthesized(usize, Result<(Vec<f32>, u32), String>),
    Spoken(usize, Result<(), String>),
    DeviceFailed(String),
}

/// 一次回答的边生成边念。方法都取 `&self`，可以放进 `Arc` 在别处 `cancel`。
pub struct SpeechStream {
    shared: Arc<Shared>,
    msgs: mpsc::Sender<Msg>,
    segmenter: Mutex<Segmenter>,
    done_rx: watch::Receiver<bool>,
    speak_output: Option<Arc<dyn SpeechOutput>>,
}

impl SpeechStream {
    /// 开始一次回答，用系统默认输出设备。
    pub fn start(engine: StreamEngine, options: StreamOptions) -> Result<Self, String> {
        Self::with_device(engine, options, Arc::new(DefaultOutputDevice))
    }

    /// 同 [`start`](Self::start)，指定输出设备（测试用假设备）。
    pub fn with_device(
        engine: StreamEngine,
        options: StreamOptions,
        device: Arc<dyn AudioDevice>,
    ) -> Result<Self, String> {
        if options.volume > 100 {
            return Err("音量必须在 0 到 100 之间".into());
        }
        let (done_tx, done_rx) = watch::channel(false);
        let shared = Arc::new(Shared {
            queue: Arc::new(PcmQueue::new(options.volume)),
            state: Mutex::new(State::default()),
            done: Condvar::new(),
            done_tx,
            device_open: Mutex::new(false),
            device_closed: Condvar::new(),
            held: Mutex::new(options.held),
            released: Condvar::new(),
            started: Instant::now(),
        });
        let (msgs, inbox) = mpsc::channel();
        let speak_output = match &engine {
            StreamEngine::Speak(output) => Some(output.clone()),
            _ => None,
        };
        let coordinator = Coordinator {
            shared: shared.clone(),
            msgs: msgs.clone(),
            device,
            options,
            playback: None,
            in_flight: VecDeque::new(),
            waiting: VecDeque::new(),
            input_done: false,
            aborted: false,
            backend: Backend::Pending,
        };
        std::thread::Builder::new()
            .name("kotone-speech-stream".into())
            .spawn(move || coordinator.run(engine, inbox))
            .map_err(|e| format!("无法启动语音线程：{e}"))?;
        Ok(Self {
            shared,
            msgs,
            segmenter: Mutex::new(Segmenter::new()),
            done_rx,
            speak_output,
        })
    }

    /// 追加一段模型输出（不阻塞）。凑够一句就开始合成。`finish` / `cancel` 之后忽略。
    pub fn push_text(&self, delta: &str) {
        let mut segmenter = self.segmenter.lock().unwrap();
        if self.closed() {
            return;
        }
        for sentence in segmenter.push(delta) {
            self.enqueue(sentence);
        }
    }

    /// 回答写完了：念完（或出错、被取消）才返回。可以多次调用。
    pub async fn finish(&self) -> Result<(), String> {
        self.close_input();
        let mut done = self.done_rx.clone();
        let _ = done.wait_for(|done| *done).await;
        self.result()
    }

    /// 同 [`finish`](Self::finish)，阻塞等待，最多 `timeout`（超时不取消，返回错误）。
    pub fn finish_blocking(&self, timeout: Duration) -> Result<(), String> {
        self.close_input();
        let deadline = Instant::now() + timeout;
        let mut state = self.shared.state.lock().unwrap();
        while state.result.is_none() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("语音播放超时".into());
            }
            state = self.shared.done.wait_timeout(state, remaining).unwrap().0;
        }
        state.result.clone().unwrap()
    }

    /// 可以出声了（[`StreamOptions::held`] 时）：合成好的声音马上开始播。可以多次调用。
    pub fn release_playback(&self) {
        *self.shared.held.lock().unwrap() = false;
        self.shared.released.notify_all();
    }

    /// 临时压低 / 恢复音量（0–1）。只对流式播放的声音有效（系统语音整句念的不管）。
    pub fn set_gain(&self, gain: f32) {
        self.shared.queue.set_gain(gain);
    }

    /// 立刻停：不再合成、丢掉没播的声音，阻塞到输出设备关掉（最多 2 秒）。
    /// 之后 [`spoken_text`](Self::spoken_text) 就是玩家实际听到的部分。
    pub fn cancel(&self) -> Result<(), String> {
        {
            let mut state = self.shared.state.lock().unwrap();
            if state.result.is_some() {
                return Ok(());
            }
            state.cancelled = true;
            state.input_closed = true;
        }
        self.shared.queue.stop();
        self.shared.released.notify_all();
        let _ = self.msgs.send(Msg::Cancel);
        if let Some(output) = &self.speak_output {
            output.cancel()?;
        }
        let deadline = Instant::now() + CANCEL_TIMEOUT;
        let mut open = self.shared.device_open.lock().unwrap();
        while *open {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("语音输出尚未停止，请稍后再开始录音".into());
            }
            open = self
                .shared
                .device_closed
                .wait_timeout(open, remaining)
                .unwrap()
                .0;
        }
        Ok(())
    }

    /// 已经**完整**念出来的句子（模型原文，含标点）。被跳过的句子（内容审核）不算。
    pub fn spoken_text(&self) -> String {
        let consumed = self.shared.queue.consumed();
        let state = self.shared.state.lock().unwrap();
        state
            .entries
            .iter()
            .filter(|e| !e.failed && (e.spoken || e.end.is_some_and(|end| consumed >= end)))
            .map(|e| e.sentence.raw.as_str())
            .collect()
    }

    /// 已经**开始**念的句子（模型原文）：气泡跟着念到哪句显示到哪句。被跳过的句子不算。
    pub fn started_text(&self) -> String {
        let consumed = self.shared.queue.consumed();
        let state = self.shared.state.lock().unwrap();
        let mut text = String::new();
        // 这句声音在队列里开始的位置 = 前一句（没被跳过的）结束的位置。
        let mut start = 0;
        for entry in state.entries.iter().filter(|e| !e.failed) {
            let begun = if self.speak_output.is_some() {
                entry.started || entry.spoken
            } else {
                consumed > start
            };
            if !begun {
                break;
            }
            text.push_str(&entry.sentence.raw);
            match entry.end {
                Some(end) => start = end,
                // 正在边收边播的这一句：后面的句子还没进队列
                None if self.speak_output.is_none() => break,
                None => {}
            }
        }
        text
    }

    /// 到目前为止收到的全部原文（包括还没凑成一句的部分）。
    pub fn full_text(&self) -> String {
        let segmenter = self.segmenter.lock().unwrap();
        let state = self.shared.state.lock().unwrap();
        let mut text: String = state
            .entries
            .iter()
            .map(|e| e.sentence.raw.as_str())
            .collect();
        text.push_str(segmenter.pending());
        text
    }

    /// 第一句开始出声的时刻（声音交给输出设备；系统语音为开始 `speak` 的时刻）。
    pub fn first_audio_at(&self) -> Option<Instant> {
        self.shared
            .queue
            .first_audio_at()
            .or(self.shared.state.lock().unwrap().first_spoken)
    }

    /// [`start`](Self::start) 的时刻。
    pub fn started_at(&self) -> Instant {
        self.shared.started
    }

    /// 已经播出去的毫秒数（系统语音为 0）。
    pub fn played_ms(&self) -> u64 {
        self.shared.queue.played_ms()
    }

    /// 结束了没有（念完、出错或取消）。
    pub fn is_done(&self) -> bool {
        self.shared.state.lock().unwrap().result.is_some()
    }

    fn closed(&self) -> bool {
        self.shared.state.lock().unwrap().input_closed
    }

    fn enqueue(&self, sentence: Sentence) {
        let index = {
            let mut state = self.shared.state.lock().unwrap();
            state.entries.push(Entry {
                sentence,
                end: None,
                spoken: false,
                started: false,
                failed: false,
            });
            state.entries.len() - 1
        };
        let _ = self.msgs.send(Msg::Sentence(index));
    }

    fn close_input(&self) {
        let mut segmenter = self.segmenter.lock().unwrap();
        {
            let mut state = self.shared.state.lock().unwrap();
            if state.input_closed {
                return;
            }
            state.input_closed = true;
        }
        if let Some(rest) = segmenter.finish() {
            self.enqueue(rest);
        }
        let _ = self.msgs.send(Msg::InputDone);
    }

    fn result(&self) -> Result<(), String> {
        self.shared
            .state
            .lock()
            .unwrap()
            .result
            .clone()
            .unwrap_or_else(|| Err("语音播放还没有结束".into()))
    }
}

impl Drop for SpeechStream {
    fn drop(&mut self) {
        // 句柄丢了就没人能再取消：没念完的直接停掉（不等设备）。
        let unfinished = {
            let mut state = self.shared.state.lock().unwrap();
            let unfinished = state.result.is_none();
            if unfinished {
                state.cancelled = true;
                state.input_closed = true;
            }
            unfinished
        };
        if unfinished {
            self.shared.queue.stop();
            self.shared.released.notify_all();
            let _ = self.msgs.send(Msg::Cancel);
            if let Some(output) = &self.speak_output {
                let _ = output.cancel();
            }
        }
    }
}

/// 一句正在合成（或合成好了在等前面的句子播）。
struct InFlight {
    index: usize,
    session: Option<(u64, DeepseekStream)>,
    audio: Vec<f32>,
    rate: u32,
    done: bool,
    failed: bool,
}

enum Backend {
    Pending,
    Deepseek {
        config: OnlineConfig,
        spare: Option<(u64, DeepseekStream, Instant)>,
        next_id: u64,
    },
    /// 合成线程：逐句 `synthesize`，声音交回协调线程排进播放队列。
    Synthesize(mpsc::Sender<usize>),
    /// 系统语音线程：逐句 `speak`，自己出声。
    Speak(mpsc::Sender<usize>),
}

struct Coordinator {
    shared: Arc<Shared>,
    msgs: mpsc::Sender<Msg>,
    device: Arc<dyn AudioDevice>,
    options: StreamOptions,
    playback: Option<JoinHandle<()>>,
    in_flight: VecDeque<InFlight>,
    waiting: VecDeque<usize>,
    input_done: bool,
    aborted: bool,
    backend: Backend,
}

impl Coordinator {
    fn run(mut self, engine: StreamEngine, inbox: mpsc::Receiver<Msg>) {
        let label = engine.label();
        match engine {
            StreamEngine::Deepseek(config) => {
                self.backend = Backend::Deepseek {
                    config,
                    spare: None,
                    next_id: 1,
                };
                self.open_spare();
            }
            StreamEngine::Synthesize(output) => {
                self.backend = Backend::Synthesize(self.spawn_synth_worker(output));
            }
            StreamEngine::Speak(output) => {
                self.backend = Backend::Speak(self.spawn_speak_worker(output));
            }
        }
        let cancelled = loop {
            match inbox.recv_timeout(Duration::from_millis(100)) {
                Ok(Msg::Cancel) | Err(mpsc::RecvTimeoutError::Disconnected) => break true,
                Ok(msg) => self.handle(msg),
                Err(mpsc::RecvTimeoutError::Timeout) => self.refresh_spare(),
            }
            self.dispatch();
            self.drain_head();
            if self.input_done && self.waiting.is_empty() && self.in_flight.is_empty() {
                break false;
            }
        };
        self.close_sessions();
        if cancelled {
            self.shared.queue.stop();
        } else {
            self.shared.queue.finish_input();
        }
        if let Some(playback) = self.playback.take() {
            let _ = playback.join();
        }
        let (result, sentences, first_spoken) = {
            let mut state = self.shared.state.lock().unwrap();
            let result = if state.cancelled || cancelled {
                Err(CANCELLED.to_string())
            } else if let Some(error) = state.error.clone() {
                Err(error)
            } else {
                Ok(())
            };
            state.result = Some(result.clone());
            (result, state.entries.len(), state.first_spoken)
        };
        let first = self
            .shared
            .queue
            .first_audio_at()
            .or(first_spoken)
            .map(|at| at.duration_since(self.shared.started).as_millis());
        kotone_core::log::log(&format!(
            "speech stream ({label}): {sentences} sentences, first audio {first:?} ms after start, \
             played {} ms, total {} ms, {result:?}",
            self.shared.queue.played_ms(),
            self.shared.started.elapsed().as_millis(),
        ));
        self.shared.done.notify_all();
        self.shared.done_tx.send_replace(true);
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Sentence(index) => {
                if self.aborted {
                    self.mark_failed(index);
                } else {
                    self.waiting.push_back(index);
                }
            }
            Msg::InputDone => {
                self.input_done = true;
                if self.waiting.is_empty() {
                    self.drop_spare();
                }
            }
            Msg::Cancel => {}
            Msg::Deepseek(id, event) => self.on_deepseek(id, event),
            Msg::Synthesized(index, result) => {
                let Some(item) = self.in_flight.iter_mut().find(|i| i.index == index) else {
                    return;
                };
                item.done = true;
                match result {
                    Ok((samples, rate)) => {
                        item.audio = samples;
                        item.rate = rate;
                    }
                    Err(error) => {
                        item.failed = true;
                        self.abort(error);
                    }
                }
            }
            Msg::Spoken(index, result) => {
                let Some(position) = self.in_flight.iter().position(|i| i.index == index) else {
                    return;
                };
                self.in_flight.remove(position);
                match result {
                    Ok(()) => self.shared.state.lock().unwrap().entries[index].spoken = true,
                    Err(error) => {
                        self.mark_failed(index);
                        if error != CANCELLED {
                            self.abort(error);
                        }
                    }
                }
            }
            Msg::DeviceFailed(error) => self.abort(error),
        }
    }

    fn on_deepseek(&mut self, id: u64, event: StreamEvent) {
        if let Backend::Deepseek { spare, .. } = &mut self.backend {
            if spare
                .as_ref()
                .is_some_and(|(spare_id, _, _)| *spare_id == id)
            {
                // 预先连好的那一路还没用就出错了（网络、鉴权、60 秒超时）：丢掉，
                // 下一句用时重新连，错误交给那一句报告。
                if let StreamEvent::Failed(error) | StreamEvent::Rejected(error) = event {
                    kotone_core::log::log(&format!(
                        "speech stream: spare session dropped: {error}"
                    ));
                    *spare = None;
                }
                return;
            }
        }
        let Some(item) = self
            .in_flight
            .iter_mut()
            .find(|i| i.session.as_ref().is_some_and(|(sid, _)| *sid == id))
        else {
            return;
        };
        match event {
            StreamEvent::Audio(samples) => {
                item.rate = SAMPLE_RATE;
                item.audio
                    .extend(samples.iter().map(|s| *s as f32 / 32768.0));
            }
            StreamEvent::Finished => {
                item.done = true;
                item.session = None;
            }
            StreamEvent::Rejected(error) => {
                // 只是这句不能念（内容审核等）：跳过这句，接着念后面的。
                item.done = true;
                item.failed = true;
                item.audio.clear();
                item.session = None;
                self.shared.fail(error);
            }
            StreamEvent::Failed(error) => {
                item.done = true;
                item.failed = true;
                item.session = None;
                self.abort(error);
            }
        }
    }

    /// 出了不是这一句本身的问题：后面的都不合成了，已经排进播放队列的照常播完。
    fn abort(&mut self, error: String) {
        self.shared.fail(error);
        self.aborted = true;
        for index in std::mem::take(&mut self.waiting) {
            self.mark_failed(index);
        }
        for item in &mut self.in_flight {
            if !item.done {
                item.done = true;
                item.failed = true;
                if let Some((_, session)) = item.session.take() {
                    session.cancel();
                }
            }
        }
        // 系统语音模式下还没念的句子：等 Spoken 回来时已经标成失败
        if matches!(self.backend, Backend::Speak(_)) {
            for item in std::mem::take(&mut self.in_flight) {
                self.mark_failed(item.index);
            }
        }
        self.drop_spare();
    }

    fn mark_failed(&self, index: usize) {
        if let Some(entry) = self.shared.state.lock().unwrap().entries.get_mut(index) {
            entry.failed = true;
        }
    }

    /// 合成好还没播的声音有多长。
    fn unplayed(&self) -> Duration {
        let queue = &self.shared.queue;
        let unplayed = queue.appended().saturating_sub(queue.consumed());
        Duration::from_millis(unplayed * 1000 / queue.rate().max(1) as u64)
    }

    /// 能合成就把等着的句子发出去。
    fn dispatch(&mut self) {
        while !self.aborted && !self.waiting.is_empty() {
            let speak = matches!(self.backend, Backend::Speak(_));
            if !speak && (self.in_flight.len() >= MAX_IN_FLIGHT || self.unplayed() > LOOKAHEAD) {
                return;
            }
            let index = self.waiting.pop_front().unwrap();
            // 合成费用归这一轮：DeepSeek 按发出去的文字记账，其他在线引擎按请求的文字记账，
            // 都是这句的 `speak`，整句认领。
            if let Some(owner) = &self.options.owner {
                let text = self.shared.state.lock().unwrap().entries[index]
                    .sentence
                    .speak
                    .clone();
                kotone_core::usage::claim_speech(&text, owner.clone());
            }
            let mut item = InFlight {
                index,
                session: None,
                audio: Vec::new(),
                rate: 0,
                done: false,
                failed: false,
            };
            match &self.backend {
                Backend::Synthesize(jobs) | Backend::Speak(jobs) => {
                    let _ = jobs.send(index);
                }
                Backend::Deepseek { .. } => {
                    let text = self.shared.state.lock().unwrap().entries[index]
                        .sentence
                        .speak
                        .clone();
                    match self.take_session() {
                        Ok((id, session)) => {
                            session.send_text(&text);
                            session.finish_text();
                            item.session = Some((id, session));
                        }
                        Err(error) => {
                            self.mark_failed(index);
                            self.abort(error);
                            return;
                        }
                    }
                }
                Backend::Pending => return,
            }
            self.in_flight.push_back(item);
            if !speak {
                self.ensure_playback();
            }
            // 还会有下一句：先连好下一路
            if !self.input_done || !self.waiting.is_empty() {
                self.open_spare();
            }
        }
    }

    /// 把队首（以及后面已经合成好的）句子的声音按顺序放进播放队列。
    fn drain_head(&mut self) {
        while let Some(head) = self.in_flight.front_mut() {
            if !head.audio.is_empty() && !head.failed {
                let audio = std::mem::take(&mut head.audio);
                self.shared.queue.append(&audio, head.rate);
            }
            if !head.done {
                return;
            }
            let head = self.in_flight.pop_front().unwrap();
            let end = self.shared.queue.appended();
            let mut state = self.shared.state.lock().unwrap();
            let entry = &mut state.entries[head.index];
            if head.failed {
                entry.failed = true;
            } else {
                entry.end = Some(end);
            }
        }
    }

    fn take_session(&mut self) -> Result<(u64, DeepseekStream), String> {
        let Backend::Deepseek {
            config,
            spare,
            next_id,
        } = &mut self.backend
        else {
            return Err("内部错误：不是 DeepSeek 会话".into());
        };
        if let Some((id, session, at)) = spare.take() {
            if at.elapsed() < SPARE_MAX_AGE {
                return Ok((id, session));
            }
            session.cancel();
        }
        open_session(&self.msgs, &self.options, config, next_id)
    }

    fn open_spare(&mut self) {
        let Backend::Deepseek {
            config,
            spare,
            next_id,
        } = &mut self.backend
        else {
            return;
        };
        if spare.is_some() {
            return;
        }
        match open_session(&self.msgs, &self.options, config, next_id) {
            Ok((id, session)) => *spare = Some((id, session, Instant::now())),
            Err(error) => {
                kotone_core::log::log(&format!("speech stream: cannot pre-connect: {error}"))
            }
        }
    }

    fn drop_spare(&mut self) {
        if let Backend::Deepseek { spare, .. } = &mut self.backend {
            if let Some((_, session, _)) = spare.take() {
                session.cancel();
            }
        }
    }

    fn refresh_spare(&mut self) {
        let stale = matches!(
            &self.backend,
            Backend::Deepseek { spare: Some((_, _, at)), .. } if at.elapsed() >= SPARE_MAX_AGE
        );
        if stale {
            self.drop_spare();
            if !self.input_done && !self.aborted {
                self.open_spare();
            }
        }
    }

    fn close_sessions(&mut self) {
        for item in &mut self.in_flight {
            if let Some((_, session)) = item.session.take() {
                session.cancel();
            }
        }
        self.drop_spare();
        // 工作线程的任务通道随 backend 一起丢掉，线程做完手上这句就退出。
        self.backend = Backend::Pending;
    }

    /// 第一次有句子要合成时起播放线程（设备等有声音才打开）。
    fn ensure_playback(&mut self) {
        if self.playback.is_some() {
            return;
        }
        let shared = self.shared.clone();
        let device = self.device.clone();
        let msgs = self.msgs.clone();
        self.playback = std::thread::Builder::new()
            .name("kotone-speech-playback".into())
            .spawn(move || {
                if let Err(error) = playback(&shared, device.as_ref()) {
                    kotone_core::log::log(&format!("speech stream: {error}"));
                    // 协调线程可能已经在等播放线程结束：错误直接记下，再通知它别再合成。
                    shared.fail(error.clone());
                    shared.queue.stop();
                    let _ = msgs.send(Msg::DeviceFailed(error));
                }
            })
            .ok();
    }

    fn spawn_synth_worker(&self, output: Arc<dyn SpeechOutput>) -> mpsc::Sender<usize> {
        let (jobs, receiver) = mpsc::channel::<usize>();
        let shared = self.shared.clone();
        let msgs = self.msgs.clone();
        let options = self.options.clone();
        let _ = std::thread::Builder::new()
            .name("kotone-speech-synth".into())
            .spawn(move || {
                for index in receiver {
                    if shared.queue.is_stopped() {
                        return;
                    }
                    // 已经出错放弃了：排着的句子不再合成
                    if shared.state.lock().unwrap().error.is_some() {
                        continue;
                    }
                    let request = request_for(&shared, &options, index);
                    let result = synthesize_with_retry(output.as_ref(), request, &shared);
                    if let Err(error) = &result {
                        // 先自己记下，排在后面的句子就不再合成（不等协调线程处理消息）
                        shared.fail(error.clone());
                    }
                    if msgs.send(Msg::Synthesized(index, result)).is_err() {
                        return;
                    }
                }
            });
        jobs
    }

    fn spawn_speak_worker(&self, output: Arc<dyn SpeechOutput>) -> mpsc::Sender<usize> {
        let (jobs, receiver) = mpsc::channel::<usize>();
        let shared = self.shared.clone();
        let msgs = self.msgs.clone();
        let options = self.options.clone();
        let ticket = output.generation();
        let _ = std::thread::Builder::new()
            .name("kotone-speech-speak".into())
            .spawn(move || {
                for index in receiver {
                    {
                        let state = shared.state.lock().unwrap();
                        if state.cancelled {
                            return;
                        }
                        // 已经出错放弃了：排着的句子不再念
                        if state.error.is_some() {
                            continue;
                        }
                    }
                    // 前面还有别的话在播：等轮到它
                    if !shared.wait_released() {
                        return;
                    }
                    let request = request_for(&shared, &options, index);
                    {
                        let mut state = shared.state.lock().unwrap();
                        if state.cancelled {
                            return;
                        }
                        state.first_spoken.get_or_insert_with(Instant::now);
                        state.entries[index].started = true;
                    }
                    let result = output.speak_if_current(request, ticket);
                    if let Err(error) = &result {
                        if error != CANCELLED {
                            shared.fail(error.clone());
                        }
                    }
                    if msgs.send(Msg::Spoken(index, result)).is_err() {
                        return;
                    }
                }
            });
        jobs
    }
}

fn open_session(
    msgs: &mpsc::Sender<Msg>,
    options: &StreamOptions,
    config: &OnlineConfig,
    next_id: &mut u64,
) -> Result<(u64, DeepseekStream), String> {
    let id = *next_id;
    *next_id += 1;
    let tx = msgs.clone();
    let session = DeepseekStream::open(config, options.voice_id.as_deref(), move |event| {
        let _ = tx.send(Msg::Deepseek(id, event));
    })?;
    Ok((id, session))
}

fn request_for(shared: &Shared, options: &StreamOptions, index: usize) -> SpeechRequest {
    SpeechRequest {
        text: shared.state.lock().unwrap().entries[index]
            .sentence
            .speak
            .clone(),
        voice_id: options.voice_id.clone(),
        volume: options.volume,
        rate: options.rate,
    }
}

/// 本地语音的工作线程队列很短（预取填充话时可能满）：满了稍等再试，最多 3 秒。
fn synthesize_with_retry(
    output: &dyn SpeechOutput,
    request: SpeechRequest,
    shared: &Shared,
) -> Result<(Vec<f32>, u32), String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match output.synthesize(request.clone()) {
            Err(error) if error.contains("队列已满") && Instant::now() < deadline => {
                if shared.queue.is_stopped() {
                    return Err(CANCELLED.into());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            other => return other,
        }
    }
}

/// 播放线程：有声音就开设备播，空闲久了先关设备，播完或被停就退出。
fn playback(shared: &Shared, device: &dyn AudioDevice) -> Result<(), String> {
    let queue = &shared.queue;
    // 前面还有别的话在播：先不开设备（合成照常进行）
    if !shared.wait_released() {
        return Ok(());
    }
    loop {
        while !queue.ready() {
            queue.wait(Duration::from_millis(100));
        }
        {
            let mut open = shared.device_open.lock().unwrap();
            if queue.is_stopped() || queue.is_drained() {
                return Ok(());
            }
            *open = true;
        }
        let end = device.play(queue);
        *shared.device_open.lock().unwrap() = false;
        shared.device_closed.notify_all();
        match end? {
            PlayEnd::Finished | PlayEnd::Stopped => return Ok(()),
            PlayEnd::Idle => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::tests::{config, fake_server, Script};
    use crate::stream_player::fake::FakeDevice;
    use kotone_core::speech::SpeechVoice;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    fn options() -> StreamOptions {
        StreamOptions {
            volume: 100,
            ..StreamOptions::default()
        }
    }

    fn push_in_pieces(stream: &SpeechStream, text: &str, piece: usize) {
        let chars: Vec<char> = text.chars().collect();
        for chunk in chars.chunks(piece) {
            stream.push_text(&chunk.iter().collect::<String>());
        }
    }

    /// 播出来的声音按样本值分段：(值, 样本数)。
    fn runs(samples: &[f32]) -> Vec<(i32, usize)> {
        let mut out: Vec<(i32, usize)> = Vec::new();
        for sample in samples {
            let value = (sample * 32768.0).round() as i32;
            match out.last_mut() {
                Some((last, count)) if *last == value => *count += 1,
                _ => out.push((value, 1)),
            }
        }
        out
    }

    #[test]
    fn deepseek_sentences_play_in_order_and_all_count_as_spoken() {
        let (base, seen) = fake_server(Script {
            delay_per_char: Duration::from_millis(3),
            ..Script::echo(240)
        });
        let device = Arc::new(FakeDevice::new(20.0));
        let stream = SpeechStream::with_device(
            StreamEngine::Deepseek(config(&base)),
            options(),
            device.clone(),
        )
        .unwrap();
        let text = "好的，我来看看。这一句比较长，会合成得慢一些。**短句**。";
        push_in_pieces(&stream, text, 3);
        stream.finish_blocking(Duration::from_secs(10)).unwrap();
        // 每句的声音按句子顺序播，句与句之间不交错（长句合成慢也不会被后面的短句插队）
        let played = device.played.lock().unwrap().clone();
        assert_eq!(
            runs(&played),
            vec![(300, 720), (500, 1200), (1500, 3600), (300, 720)]
        );
        assert_eq!(stream.spoken_text(), text);
        assert_eq!(stream.full_text(), text);
        assert!(stream.first_audio_at().unwrap() > stream.started_at());
        assert!(stream.is_done());
        let seen = seen.lock().unwrap();
        let mut texts = seen.texts.clone();
        texts.sort();
        let mut expected = vec![
            "好的，",
            "我来看看。",
            "这一句比较长，会合成得慢一些。",
            "短句。",
        ];
        expected.sort();
        assert_eq!(texts, expected);
        assert_eq!(seen.finished_text, 4);
        // 每句一路会话，外加至多一路预先连好、最后没用上的
        assert!(
            seen.connections >= 4 && seen.connections <= 5,
            "{}",
            seen.connections
        );
    }

    #[test]
    fn cancel_mid_sentence_keeps_only_fully_played_sentences() {
        let (base, _) = fake_server(Script::echo(2400));
        let device = Arc::new(FakeDevice::new(1.0));
        let stream = SpeechStream::with_device(
            StreamEngine::Deepseek(config(&base)),
            options(),
            device.clone(),
        )
        .unwrap();
        // 第一句 0.3 秒，第二句 1 秒
        stream.push_text("好的，我来看看对面的出装。还有一句。");
        let deadline = Instant::now() + Duration::from_secs(5);
        while stream.played_ms() < 500 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let started = Instant::now();
        stream.cancel().unwrap();
        assert!(started.elapsed() < Duration::from_millis(300));
        let played = stream.played_ms();
        assert!((500..1300).contains(&played), "{played}");
        assert_eq!(stream.spoken_text(), "好的，");
        assert_eq!(
            stream.finish_blocking(Duration::from_secs(2)).unwrap_err(),
            CANCELLED
        );
        // 取消以后再也不出声
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(stream.played_ms(), played);
        stream.push_text("不会再念。");
        assert_eq!(stream.spoken_text(), "好的，");
    }

    #[test]
    fn a_rejected_sentence_is_skipped_and_reported() {
        let (base, _) = fake_server(Script {
            filter_word: Some("违禁"),
            ..Script::echo(24)
        });
        let device = Arc::new(FakeDevice::new(20.0));
        let stream =
            SpeechStream::with_device(StreamEngine::Deepseek(config(&base)), options(), device)
                .unwrap();
        stream.push_text("好的，这句有违禁词。最后一句。");
        let error = stream.finish_blocking(Duration::from_secs(10)).unwrap_err();
        assert!(error.contains("内容审核"), "{error}");
        assert_eq!(stream.spoken_text(), "好的，最后一句。");
    }

    #[test]
    fn a_rejected_key_fails_fast_without_opening_the_device() {
        let (base, _) = fake_server(Script {
            reject: true,
            ..Script::echo(24)
        });
        let device = Arc::new(FakeDevice::new(20.0));
        let stream = SpeechStream::with_device(
            StreamEngine::Deepseek(config(&base)),
            options(),
            device.clone(),
        )
        .unwrap();
        stream.push_text("你好，在吗？我在。");
        let error = stream.finish_blocking(Duration::from_secs(10)).unwrap_err();
        assert!(error.contains("Key 无效"), "{error}");
        assert!(!error.contains("sk-test"));
        assert_eq!(stream.spoken_text(), "");
        assert_eq!(device.opened.load(Ordering::SeqCst), 0);
        assert!(stream.first_audio_at().is_none());
    }

    /// 能拿到声音数据的假引擎：16 kHz，每字 160 个样本，样本值 = 字数 / 100。
    #[derive(Default)]
    struct FakeOutput {
        epoch: AtomicU64,
        spoken: Mutex<Vec<String>>,
        synthesized: Mutex<Vec<(String, i32)>>,
        cancelled: AtomicBool,
        busy_once: AtomicBool,
    }

    impl SpeechOutput for FakeOutput {
        fn voices(&self) -> Result<Vec<SpeechVoice>, String> {
            Ok(Vec::new())
        }
        fn generation(&self) -> u64 {
            self.epoch.load(Ordering::SeqCst)
        }
        fn speak_if_current(&self, request: SpeechRequest, ticket: u64) -> Result<(), String> {
            for _ in 0..20 {
                if self.generation() != ticket {
                    return Err(CANCELLED.into());
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            self.spoken.lock().unwrap().push(request.text);
            Ok(())
        }
        fn cancel(&self) -> Result<(), String> {
            self.cancelled.store(true, Ordering::SeqCst);
            self.epoch.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn synthesize(&self, request: SpeechRequest) -> Result<(Vec<f32>, u32), String> {
            // 第一次假装队列满，应当稍等重试
            if !self.busy_once.swap(true, Ordering::SeqCst) {
                return Err("本地语音队列已满或服务已停止".into());
            }
            let chars = request.text.chars().count();
            self.synthesized
                .lock()
                .unwrap()
                .push((request.text, request.rate));
            Ok((vec![chars as f32 / 100.0; chars * 160], 16_000))
        }
    }

    #[test]
    fn synthesizing_engines_queue_sentences_and_resample_to_one_rate() {
        let output = Arc::new(FakeOutput::default());
        let device = Arc::new(FakeDevice::new(20.0));
        let stream = SpeechStream::with_device(
            StreamEngine::Synthesize(output.clone()),
            StreamOptions {
                rate: 3,
                ..options()
            },
            device.clone(),
        )
        .unwrap();
        stream.push_text("收到，马上就去。");
        stream.push_text("等我十秒");
        stream.finish_blocking(Duration::from_secs(10)).unwrap();
        assert_eq!(
            *output.synthesized.lock().unwrap(),
            vec![
                ("收到，".to_string(), 3),
                ("马上就去。".to_string(), 3),
                ("等我十秒".to_string(), 3)
            ]
        );
        // 16 kHz → 队列 16 kHz（第一句决定采样率），每字 160 个样本
        assert_eq!(device.played.lock().unwrap().len(), (3 + 5 + 4) * 160);
        assert_eq!(stream.spoken_text(), "收到，马上就去。等我十秒");
        assert_eq!(device.opened.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn system_voice_speaks_sentence_by_sentence_and_cancel_stops_it() {
        let output = Arc::new(FakeOutput::default());
        let device = Arc::new(FakeDevice::new(20.0));
        let stream = SpeechStream::with_device(
            StreamEngine::Speak(output.clone()),
            options(),
            device.clone(),
        )
        .unwrap();
        stream.push_text("第一句话。第二句话。第三句话。");
        let deadline = Instant::now() + Duration::from_secs(5);
        while output.spoken.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        // 等协调线程收到第一句念完的消息
        while stream.spoken_text().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        stream.cancel().unwrap();
        assert!(output.cancelled.load(Ordering::SeqCst));
        assert_eq!(
            stream.finish_blocking(Duration::from_secs(2)).unwrap_err(),
            CANCELLED
        );
        assert_eq!(stream.spoken_text(), "第一句话。");
        assert!(stream.first_audio_at().is_some());
        assert_eq!(device.opened.load(Ordering::SeqCst), 0);
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(output.spoken.lock().unwrap().len(), 1);
    }

    /// 系统语音念到一半出错：后面排着的句子不再念。
    struct FailingVoice(Mutex<Vec<String>>);

    impl SpeechOutput for FailingVoice {
        fn voices(&self) -> Result<Vec<SpeechVoice>, String> {
            Ok(Vec::new())
        }
        fn generation(&self) -> u64 {
            0
        }
        fn speak_if_current(&self, request: SpeechRequest, _: u64) -> Result<(), String> {
            let mut spoken = self.0.lock().unwrap();
            spoken.push(request.text);
            if spoken.len() == 2 {
                return Err("系统语音出错了".into());
            }
            Ok(())
        }
        fn cancel(&self) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn a_system_voice_error_stops_the_remaining_sentences() {
        let output = Arc::new(FailingVoice(Mutex::new(Vec::new())));
        let stream = SpeechStream::with_device(
            StreamEngine::Speak(output.clone()),
            options(),
            Arc::new(FakeDevice::new(20.0)),
        )
        .unwrap();
        stream.push_text("第一句话。第二句话。第三句话。第四句话。");
        let error = stream.finish_blocking(Duration::from_secs(5)).unwrap_err();
        assert_eq!(error, "系统语音出错了");
        assert_eq!(stream.spoken_text(), "第一句话。");
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(output.0.lock().unwrap().len(), 2);
    }

    #[test]
    fn system_voice_finishes_normally() {
        let output = Arc::new(FakeOutput::default());
        let stream = SpeechStream::with_device(
            StreamEngine::Speak(output.clone()),
            options(),
            Arc::new(FakeDevice::new(20.0)),
        )
        .unwrap();
        stream.push_text("好的，");
        stream.push_text("知道了");
        stream.finish_blocking(Duration::from_secs(5)).unwrap();
        assert_eq!(*output.spoken.lock().unwrap(), vec!["好的，", "知道了"]);
        assert_eq!(stream.spoken_text(), "好的，知道了");
    }

    #[test]
    fn a_device_failure_ends_the_stream_with_an_error() {
        let output = Arc::new(FakeOutput::default());
        let mut device = FakeDevice::new(20.0);
        device.fail = true;
        let stream = SpeechStream::with_device(
            StreamEngine::Synthesize(output),
            options(),
            Arc::new(device),
        )
        .unwrap();
        stream.push_text("这句放不出来。后面这句也不用合成了。");
        let error = stream.finish_blocking(Duration::from_secs(5)).unwrap_err();
        assert!(error.contains("假设备坏了"), "{error}");
        assert_eq!(stream.spoken_text(), "");
    }

    #[test]
    fn finish_can_be_awaited_and_repeated_and_late_text_is_ignored() {
        let output = Arc::new(FakeOutput::default());
        let stream = SpeechStream::with_device(
            StreamEngine::Synthesize(output),
            options(),
            Arc::new(FakeDevice::new(50.0)),
        )
        .unwrap();
        stream.push_text("马上");
        assert_eq!(stream.full_text(), "马上");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            stream.finish().await.unwrap();
            stream.finish().await.unwrap();
        });
        stream.push_text("太晚了。");
        assert_eq!(stream.full_text(), "马上");
        assert_eq!(stream.spoken_text(), "马上");
        stream.cancel().unwrap();
        assert!(stream.finish_blocking(Duration::from_secs(1)).is_ok());
    }

    #[test]
    fn nothing_to_say_finishes_immediately() {
        let (base, seen) = fake_server(Script::echo(24));
        let device = Arc::new(FakeDevice::new(20.0));
        let stream = SpeechStream::with_device(
            StreamEngine::Deepseek(config(&base)),
            options(),
            device.clone(),
        )
        .unwrap();
        stream.push_text("😀");
        stream.finish_blocking(Duration::from_secs(5)).unwrap();
        assert_eq!(stream.spoken_text(), "");
        assert_eq!(device.opened.load(Ordering::SeqCst), 0);
        assert!(seen.lock().unwrap().texts.is_empty());
    }

    #[test]
    fn a_held_stream_synthesizes_but_stays_silent_until_released() {
        let output = Arc::new(FakeOutput::default());
        let device = Arc::new(FakeDevice::new(20.0));
        let stream = SpeechStream::with_device(
            StreamEngine::Synthesize(output.clone()),
            StreamOptions {
                held: true,
                ..options()
            },
            device.clone(),
        )
        .unwrap();
        stream.push_text("前面还在说话。");
        let deadline = Instant::now() + Duration::from_secs(5);
        while output.synthesized.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        // 合成了，但没开设备、没出声
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(output.synthesized.lock().unwrap().len(), 1);
        assert_eq!(device.opened.load(Ordering::SeqCst), 0);
        assert!(stream.first_audio_at().is_none());
        assert_eq!(stream.started_text(), "");
        let released = Instant::now();
        stream.release_playback();
        stream.finish_blocking(Duration::from_secs(5)).unwrap();
        assert!(stream.first_audio_at().unwrap() >= released);
        assert_eq!(stream.spoken_text(), "前面还在说话。");
        assert_eq!(stream.started_text(), "前面还在说话。");
    }

    #[test]
    fn cancelling_a_held_stream_returns_at_once() {
        let output = Arc::new(FakeOutput::default());
        for engine in [
            StreamEngine::Synthesize(output.clone()),
            StreamEngine::Speak(output.clone()),
        ] {
            let device = Arc::new(FakeDevice::new(20.0));
            let stream = SpeechStream::with_device(
                engine,
                StreamOptions {
                    held: true,
                    ..options()
                },
                device.clone(),
            )
            .unwrap();
            stream.push_text("一直轮不到我。下一句。");
            std::thread::sleep(Duration::from_millis(50));
            let started = Instant::now();
            stream.cancel().unwrap();
            assert!(started.elapsed() < Duration::from_millis(500));
            assert_eq!(
                stream.finish_blocking(Duration::from_secs(2)).unwrap_err(),
                CANCELLED
            );
            assert_eq!(stream.spoken_text(), "");
            assert_eq!(device.opened.load(Ordering::SeqCst), 0);
        }
        // 系统语音也没念出任何一句
        assert!(output.spoken.lock().unwrap().is_empty());
    }

    #[test]
    fn started_text_follows_the_sentence_being_played() {
        let (base, _) = fake_server(Script::echo(2_400));
        // 慢速设备：每句 0.1 秒声音 × 字数，按 2 倍速播
        let device = Arc::new(FakeDevice::new(2.0));
        let stream = SpeechStream::with_device(
            StreamEngine::Deepseek(config(&base)),
            options(),
            device.clone(),
        )
        .unwrap();
        stream.push_text("第一句。第二句话。");
        stream.push_text("第三句。");
        let deadline = Instant::now() + Duration::from_secs(10);
        while stream.started_text().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        // 第一句开始念时气泡只显示第一句，还没念完
        assert_eq!(stream.started_text(), "第一句。");
        assert_eq!(stream.spoken_text(), "");
        while !stream.started_text().contains("第二句") && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(stream.started_text(), "第一句。第二句话。");
        assert_eq!(stream.spoken_text(), "第一句。");
        stream.cancel().unwrap();
    }

    #[test]
    fn every_sentence_is_billed_to_the_owner_turn() {
        // 账本是进程里唯一的：开在临时目录，只看这个测试自己那段对话的记录。
        let dir = std::env::temp_dir().join(format!("kotone-tts-billing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        kotone_core::usage::enable(dir.join("usage.jsonl"));
        let ledger = kotone_core::usage::global().unwrap().path().to_path_buf();
        let (base, _) = fake_server(Script::echo(240));
        let owner = kotone_core::usage::Owner {
            conversation: "stream-billing-test".into(),
            turn: Some(42),
        };
        let stream = SpeechStream::with_device(
            StreamEngine::Deepseek(config(&base)),
            StreamOptions {
                owner: Some(owner),
                ..options()
            },
            Arc::new(FakeDevice::new(20.0)),
        )
        .unwrap();
        let text = "计费归属测试甲。计费归属测试乙，后半句。";
        push_in_pieces(&stream, text, 4);
        stream.finish_blocking(Duration::from_secs(10)).unwrap();
        // 用量在合成线程退出前记下：稍等最后一路的线程
        let deadline = Instant::now() + Duration::from_secs(5);
        let ours = loop {
            let records: Vec<kotone_core::usage::UsageRecord> =
                kotone_core::usage::UsageLedger::new(&ledger)
                    .read_since(0)
                    .into_iter()
                    .filter(|r| r.conversation.as_deref() == Some("stream-billing-test"))
                    .collect();
            if records.len() >= 2 || Instant::now() > deadline {
                break records;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        // 两句各记一行（每句一路会话），都归第 42 轮
        assert_eq!(ours.len(), 2, "{ours:?}");
        assert!(ours
            .iter()
            .all(|r| r.turn == Some(42) && r.kind == Some(kotone_core::usage::UsageKind::Tts)));
    }

    #[test]
    fn dropping_an_unfinished_stream_stops_it() {
        let (base, _) = fake_server(Script::echo(2400));
        let device = Arc::new(FakeDevice::new(1.0));
        let stream = SpeechStream::with_device(
            StreamEngine::Deepseek(config(&base)),
            options(),
            device.clone(),
        )
        .unwrap();
        stream.push_text("这句话会念一会儿才念完。");
        let deadline = Instant::now() + Duration::from_secs(5);
        while stream.played_ms() < 100 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(stream);
        std::thread::sleep(Duration::from_millis(100));
        let played = device.played.lock().unwrap().len();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(device.played.lock().unwrap().len(), played);
    }

    /// 真接口端到端：模拟模型每 40 ms 吐两个字，假设备按实时速度“播放”（不出声）。
    /// 量从开始到第一句出声、念完的时间；再来一次中途打断，核对只记已念完的句子。
    #[test]
    #[ignore = "needs KOTONE_REAL_DEEPSEEK_KEY and network; costs a few cents"]
    fn real_deepseek_speech_stream_end_to_end() {
        let key = std::env::var("KOTONE_REAL_DEEPSEEK_KEY").expect("KOTONE_REAL_DEEPSEEK_KEY");
        let mut c = config(crate::online::DEEPSEEK_ORIGIN);
        c.api_key = key;
        let text = "好的，我来看看对面的出装。他们打野刚买了真眼，小龙那边要小心！你先别急着开团。";
        for interrupt in [false, true] {
            let device = Arc::new(FakeDevice::new(1.0));
            let stream = Arc::new(
                SpeechStream::with_device(
                    StreamEngine::Deepseek(c.clone()),
                    options(),
                    device.clone(),
                )
                .unwrap(),
            );
            // 模型在 0.6 秒后开始出字（握手藏在这段时间里）
            std::thread::sleep(Duration::from_millis(600));
            let first_text = Instant::now();
            let chars: Vec<char> = text.chars().collect();
            for piece in chars.chunks(2) {
                stream.push_text(&piece.iter().collect::<String>());
                std::thread::sleep(Duration::from_millis(40));
            }
            let text_done = first_text.elapsed();
            let result = if interrupt {
                let deadline = Instant::now() + Duration::from_secs(20);
                while stream.played_ms() < 2500 && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                stream.cancel().map(|_| ())
            } else {
                stream.finish_blocking(Duration::from_secs(60))
            };
            let first_audio = stream
                .first_audio_at()
                .map(|at| at.duration_since(first_text));
            let played = device.played.lock().unwrap().clone();
            let as_i16: Vec<i16> = played.iter().map(|s| (s * 32768.0) as i16).collect();
            let (le, swapped) = crate::stream::tests::smoothness(&as_i16);
            println!(
                "real_speech_stream interrupt={interrupt}: first_audio={first_audio:?} after the first text \
                 (text fully pushed at {text_done:?}), ended at {:?}, played {} ms, result={result:?}, \
                 smoothness {le:.0} vs swapped {swapped:.0}\n  spoken: {}",
                first_text.elapsed(),
                stream.played_ms(),
                stream.spoken_text()
            );
            if interrupt {
                result.unwrap();
                assert!(text.starts_with(&stream.spoken_text()));
                assert_ne!(stream.spoken_text(), text);
            } else {
                result.unwrap();
                assert_eq!(stream.spoken_text(), text);
            }
            assert!(first_audio.unwrap() < Duration::from_secs(3));
        }
    }
}
