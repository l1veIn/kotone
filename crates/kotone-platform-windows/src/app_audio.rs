//! 只录某一个程序的声音（ADR 021“耳朵”）：Windows 的进程回环录音
//! （`AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`，微软 ApplicationLoopback 示例，Windows 10 20348 起）。
//!
//! 照 wasapi-rs（MIT）的 `examples/record_application.rs`：按进程树录（传父进程的 PID），
//! 32 位浮点 48 kHz 立体声、事件驱动、自动格式转换；这里再混成单声道、重采样到 16 kHz 交给转写。
//! 只录指定的程序，所以琴音自己的语音、玩家的麦克风都录不进来。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

/// 同名的全部进程（最多 [`MAX_PROCESSES`] 个）。Discord、浏览器等把声音放在某个子进程里；
/// 进程回环的“含子进程”实测只覆盖直接子进程，所以逐个进程录，再由 [`Follower`] 跟着出声的那个。
pub fn pids_by_name(name: &str) -> Vec<u32> {
    crate::inject::process_list()
        .into_iter()
        .filter(|(_, _, n)| n.eq_ignore_ascii_case(name))
        .map(|(pid, _, _)| pid)
        .take(MAX_PROCESSES)
        .collect()
}

/// 一个程序最多同时录这么多个进程。
pub const MAX_PROCESSES: usize = 32;

/// 同名进程里父进程不同名的那些（每棵进程树的根）。
pub fn root_pids_by_name(name: &str) -> Vec<u32> {
    let procs = crate::inject::process_list();
    let same: Vec<&(u32, u32, String)> = procs
        .iter()
        .filter(|(_, _, n)| n.eq_ignore_ascii_case(name))
        .collect();
    let pids: Vec<u32> = same.iter().map(|(pid, _, _)| *pid).collect();
    same.iter()
        .filter(|(_, parent, _)| !pids.contains(parent))
        .map(|(pid, _, _)| *pid)
        .collect()
}

/// 只取一棵（兼容旧调用）。
pub fn root_pid_by_name(name: &str) -> Option<u32> {
    root_pids_by_name(name).into_iter().next()
}

/// 同时录好几棵进程树时，跟着正在出声的那一棵：一棵树出声后，其它树的块都丢掉，
/// 直到它安静超过 [`FOLLOW_QUIET_MS`]；跟随的那棵安静时的块照常交出去（切句要靠这些停顿）。
pub const FOLLOW_QUIET_MS: u64 = 2_000;
const LOUD: f32 = 1e-3;

#[derive(Debug, Default)]
pub struct Follower {
    active: Option<u32>,
    last_loud_ms: u64,
}

impl Follower {
    /// 这一块（来自 `pid`，时刻 `now_ms`）要不要交出去。纯逻辑，便于测试。
    pub fn accept(&mut self, pid: u32, chunk: &[f32], now_ms: u64) -> bool {
        let loud = chunk.iter().any(|x| x.abs() > LOUD);
        let idle = now_ms.saturating_sub(self.last_loud_ms) > FOLLOW_QUIET_MS;
        if loud && (self.active.is_none() || (self.active != Some(pid) && idle)) {
            self.active = Some(pid);
        }
        if self.active == Some(pid) {
            if loud {
                self.last_loud_ms = now_ms;
            }
            return true;
        }
        false
    }
}

/// 录同名程序的全部进程，只交出正在出声的那一个的声音。丢弃时全部停止。
pub struct MultiCapture {
    _captures: Vec<AppCapture>,
}

impl MultiCapture {
    pub fn start(name: &str, sink: AudioSink) -> Result<Self, String> {
        let pids = pids_by_name(name);
        if pids.is_empty() {
            return Err(format!("{name} 没有在运行"));
        }
        let shared = std::sync::Arc::new(std::sync::Mutex::new((Follower::default(), sink)));
        let started = std::time::Instant::now();
        let mut captures = Vec::new();
        let mut last_error = None;
        for pid in pids {
            let shared = shared.clone();
            match AppCapture::start_process(
                pid,
                false,
                Box::new(move |chunk: &[f32]| {
                    let now = started.elapsed().as_millis() as u64;
                    let mut guard = shared.lock().unwrap();
                    let (follower, sink) = &mut *guard;
                    if follower.accept(pid, chunk, now) {
                        sink(chunk);
                    }
                }),
            ) {
                Ok(capture) => captures.push(capture),
                Err(e) => last_error = Some(e),
            }
        }
        if captures.is_empty() {
            return Err(last_error.unwrap_or_else(|| "无法录这个程序的声音".into()));
        }
        Ok(Self {
            _captures: captures,
        })
    }
}

/// 收到 16 kHz 单声道音频块的回调（在录音线程里调用）。
pub type AudioSink = Box<dyn FnMut(&[f32]) + Send>;

/// 正在录的一个程序。丢弃时停止。
pub struct AppCapture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    error: Arc<std::sync::Mutex<Option<String>>>,
}

impl AppCapture {
    /// 开始录 `pid`（含子进程）；`on_audio` 在录音线程里收到 16 kHz 单声道的块。
    pub fn start(pid: u32, on_audio: AudioSink) -> Result<Self, String> {
        Self::start_process(pid, true, on_audio)
    }

    /// `include_tree = false`：只录这一个进程。
    pub fn start_process(
        pid: u32,
        include_tree: bool,
        mut on_audio: AudioSink,
    ) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let error = Arc::new(std::sync::Mutex::new(None));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let thread_stop = stop.clone();
        let thread_error = error.clone();
        let thread = std::thread::Builder::new()
            .name("kotone-app-audio".into())
            .spawn(move || {
                if let Err(e) = capture(pid, include_tree, &thread_stop, &ready_tx, &mut *on_audio)
                {
                    let _ = ready_tx.send(Err(e.clone()));
                    *thread_error.lock().unwrap() = Some(e);
                }
            })
            .map_err(|e| format!("无法启动录音线程：{e}"))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                stop,
                thread: Some(thread),
                error,
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err("录音线程异常退出".into())
            }
        }
    }

    /// 录音中途出的错（目标程序退出等）。
    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }

    pub fn running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }
}

impl Drop for AppCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(windows)]
fn capture(
    pid: u32,
    include_tree: bool,
    stop: &AtomicBool,
    ready: &std::sync::mpsc::Sender<Result<(), String>>,
    on_audio: &mut dyn FnMut(&[f32]),
) -> Result<(), String> {
    use std::collections::VecDeque;
    use wasapi::{AudioClient, Direction, SampleType, StreamMode, WaveFormat};

    let _ = wasapi::initialize_mta().ok();
    let format = WaveFormat::new(32, 32, SampleType::Float, 48_000, 2, None);
    let block = format.get_blockalign() as usize;
    let mut client = AudioClient::new_application_loopback_client(pid, include_tree)
        .map_err(|e| format!("无法录这个程序的声音（需要 Windows 10 20348 或更新）：{e}"))?;
    client
        .initialize_client(
            &format,
            Direction::Capture,
            StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: 0,
            },
            None,
        )
        .map_err(|e| format!("无法初始化录音：{e}"))?;
    let event = client
        .set_get_eventhandle()
        .map_err(|e| format!("无法初始化录音：{e}"))?;
    let reader = client
        .get_audiocaptureclient()
        .map_err(|e| format!("无法初始化录音：{e}"))?;
    client
        .start_stream()
        .map_err(|e| format!("无法开始录音：{e}"))?;
    let _ = ready.send(Ok(()));

    let mut queue: VecDeque<u8> = VecDeque::new();
    let mut resampler = crate::audio::Resampler::new(48_000, 16_000);
    let chunk_frames = 48_000 / 20; // 50 ms
    while !stop.load(Ordering::SeqCst) {
        let frames = reader
            .get_next_packet_size()
            .map_err(|e| format!("录音出错：{e}"))?
            .unwrap_or(0);
        if frames > 0 {
            reader
                .read_from_device_to_deque(&mut queue)
                .map_err(|e| format!("录音出错：{e}"))?;
        }
        while queue.len() >= block * chunk_frames {
            let bytes: Vec<u8> = queue.drain(..block * chunk_frames).collect();
            let mono: Vec<f32> = bytes
                .chunks_exact(block)
                .map(|frame| {
                    let l = f32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]);
                    let r = f32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]);
                    (l + r) * 0.5
                })
                .collect();
            on_audio(&resampler.process(&mono));
        }
        // 目标程序没出声时等不到事件：超时就接着检查是否要停。
        let _ = event.wait_for_event(200);
    }
    let _ = client.stop_stream();
    Ok(())
}

#[cfg(not(windows))]
fn capture(
    _pid: u32,
    _include_tree: bool,
    _stop: &AtomicBool,
    _ready: &std::sync::mpsc::Sender<Result<(), String>>,
    _on_audio: &mut dyn FnMut(&[f32]),
) -> Result<(), String> {
    Err("只支持 Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真机：录 `KOTONE_CAPTURE_APP`（缺省 python.exe）几秒，打印收到的样本数与最大振幅。
    #[test]
    fn follows_the_tree_that_is_sounding_and_keeps_its_pauses() {
        let mut f = Follower::default();
        let silent = [0.0f32; 4];
        let loud = [0.2f32; 4];
        assert!(!f.accept(1, &silent, 0), "nobody sounding yet");
        assert!(f.accept(2, &loud, 10), "first sounding tree is followed");
        assert!(
            !f.accept(1, &loud, 20),
            "another tree is ignored while the followed one is recent"
        );
        assert!(
            f.accept(2, &silent, 500),
            "pauses of the followed tree pass (needed to cut sentences)"
        );
        assert!(
            f.accept(1, &loud, 3_000),
            "after 2 s of quiet another tree can take over"
        );
        assert!(!f.accept(2, &silent, 3_100));
    }

    #[test]
    #[ignore = "needs a process that is playing audio"]
    fn captures_a_playing_process() {
        let name = std::env::var("KOTONE_CAPTURE_APP").unwrap_or_else(|_| "python.exe".into());
        let stats = Arc::new(std::sync::Mutex::new((0usize, 0f32)));
        let sink = stats.clone();
        let capture = MultiCapture::start(
            &name,
            Box::new(move |chunk: &[f32]| {
                let mut s = sink.lock().unwrap();
                s.0 += chunk.len();
                s.1 = chunk.iter().fold(s.1, |m, x| m.max(x.abs()));
            }),
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_secs(8));
        drop(capture);
        let (n, peak) = *stats.lock().unwrap();
        println!("{name}: {n} samples at 16 kHz, peak {peak:.3}");
        assert!(n > 16_000 && peak > 0.05);
    }
}
