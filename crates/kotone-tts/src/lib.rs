//! Local neural speech. One worker owns inference and output; cancellation
//! invalidates queued synthesis without waiting for a non-interruptible ONNX call.
//! Microphone admission waits only for an already-open output stream to close.

mod engine;
mod player;

pub use engine::{
    kokoro_voices, model_ready, KokoroEngine, SynthesizedSpeech, DEFAULT_VOICE, KOKORO_MODEL_DIR,
    REQUIRED_DIRS, REQUIRED_FILES,
};

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};

use kotone_core::speech::{SpeechOutput, SpeechRequest, SpeechVoice};

const CANCELLED: &str = "播报已取消";
const CANCEL_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Control {
    epoch: AtomicU64,
    stopped: AtomicBool,
    playing: Mutex<bool>,
    playback_stopped: Condvar,
    worker_finished: Mutex<bool>,
    worker_exited: Condvar,
    #[cfg(test)]
    rendered_audio: AtomicBool,
}

impl Control {
    fn current(&self, ticket: u64) -> bool {
        !self.stopped.load(Ordering::SeqCst) && self.epoch.load(Ordering::SeqCst) == ticket
    }

    fn begin_playback(self: &Arc<Self>, ticket: u64) -> Result<PlaybackLease, String> {
        let mut playing = self.playing.lock().unwrap();
        if !self.current(ticket) {
            return Err(CANCELLED.into());
        }
        if *playing {
            return Err("上一条本地播报尚未停止".into());
        }
        *playing = true;
        Ok(PlaybackLease(self.clone()))
    }

    fn cancel(&self) -> Result<(), String> {
        self.cancel_generation(None)
    }

    fn cancel_if_current(&self, ticket: u64) -> Result<(), String> {
        self.cancel_generation(Some(ticket))
    }

    fn cancel_generation(&self, expected: Option<u64>) -> Result<(), String> {
        let mut playing = self.playing.lock().unwrap();
        if expected.is_some_and(|ticket| self.epoch.load(Ordering::SeqCst) != ticket) {
            return Ok(());
        }
        self.epoch.fetch_add(1, Ordering::SeqCst);
        let deadline = Instant::now() + CANCEL_TIMEOUT;
        while *playing {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("本地语音输出尚未停止，请稍后再开始录音".into());
            }
            playing = self
                .playback_stopped
                .wait_timeout(playing, remaining)
                .unwrap()
                .0;
        }
        Ok(())
    }
}

/// Created before opening the device and released strictly after the stream drops.
struct PlaybackLease(Arc<Control>);

impl Drop for PlaybackLease {
    fn drop(&mut self) {
        *self.0.playing.lock().unwrap() = false;
        self.0.playback_stopped.notify_all();
    }
}

struct Command {
    request: SpeechRequest,
    ticket: u64,
    reply: mpsc::SyncSender<Result<(), String>>,
}

enum WorkerCommand {
    Speak(Command),
    Close,
}

pub struct LocalSpeechOutput {
    commands: mpsc::SyncSender<WorkerCommand>,
    control: Arc<Control>,
}

impl LocalSpeechOutput {
    /// Does not download or load a model, or open the sound device. The directory
    /// is the extracted kokoro-int8-multi-lang-v1_1 directory, not its parent.
    pub fn new(model_dir: PathBuf) -> Result<Self, String> {
        let (commands, receiver) = mpsc::sync_channel::<WorkerCommand>(2);
        let control = Arc::new(Control::default());
        let worker_control = control.clone();
        std::thread::Builder::new()
            .name("kotone-local-tts".into())
            .spawn(move || worker(model_dir, receiver, worker_control))
            .map_err(|error| format!("无法启动本地语音线程：{error}"))?;
        Ok(Self { commands, control })
    }

    /// True once shutdown was requested, including a close that timed out.
    /// Call close again to confirm resource release before replacing the instance.
    pub fn is_closed(&self) -> bool {
        self.control.stopped.load(Ordering::SeqCst)
    }

    /// Explicit model-directory migration barrier. Success guarantees both the
    /// output stream and neural engine have dropped. ONNX calls cannot be killed;
    /// a slow one returns an error after 10s, so callers must postpone migration.
    pub fn close(&self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        self.control.stopped.store(true, Ordering::SeqCst);
        let _ = self.control.cancel();
        let _ = self.commands.try_send(WorkerCommand::Close);
        let mut finished = self.control.worker_finished.lock().unwrap();
        while !*finished {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("本地语音推理尚未退出，请稍后再迁移模型目录".into());
            }
            finished = self
                .control
                .worker_exited
                .wait_timeout(finished, remaining)
                .unwrap()
                .0;
        }
        Ok(())
    }
}

impl SpeechOutput for LocalSpeechOutput {
    fn voices(&self) -> Result<Vec<SpeechVoice>, String> {
        Ok(kokoro_voices())
    }

    fn generation(&self) -> u64 {
        self.control.epoch.load(Ordering::SeqCst)
    }

    fn speak_if_current(&self, request: SpeechRequest, ticket: u64) -> Result<(), String> {
        request.validate()?;
        if !self.control.current(ticket) {
            return Err(CANCELLED.into());
        }
        let (reply, receiver) = mpsc::sync_channel(1);
        self.commands
            .try_send(WorkerCommand::Speak(Command {
                request,
                ticket,
                reply,
            }))
            .map_err(|_| "本地语音队列已满或服务已停止".to_string())?;
        match receiver.recv_timeout(Duration::from_secs(60)) {
            Ok(result) => result,
            Err(_) => {
                // The inference worker is still observed by the serial worker;
                // invalidate its ticket so a delayed result can never play.
                self.control.cancel_if_current(ticket)?;
                Err("本地语音生成或播放超时，已取消".into())
            }
        }
    }

    fn cancel(&self) -> Result<(), String> {
        self.control.cancel()
    }
}

impl Drop for LocalSpeechOutput {
    fn drop(&mut self) {
        self.control.stopped.store(true, Ordering::SeqCst);
        // Dropping UI state never joins an inference thread. Migration uses the
        // explicit close barrier above; a dropped instance cannot output again.
        self.control.epoch.fetch_add(1, Ordering::SeqCst);
        let _ = self.commands.try_send(WorkerCommand::Close);
    }
}

struct WorkerFinished(Arc<Control>);

impl Drop for WorkerFinished {
    fn drop(&mut self) {
        *self.0.worker_finished.lock().unwrap() = true;
        self.0.worker_exited.notify_all();
    }
}

fn worker(model_dir: PathBuf, receiver: mpsc::Receiver<WorkerCommand>, control: Arc<Control>) {
    // Created before engine so the engine is destroyed before this ack is sent.
    let _finished = WorkerFinished(control.clone());
    let mut engine: Option<KokoroEngine> = None;
    while !control.stopped.load(Ordering::SeqCst) {
        let command = match receiver.recv_timeout(Duration::from_secs(60)) {
            Ok(WorkerCommand::Speak(command)) => command,
            Ok(WorkerCommand::Close) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Free neural weights during normal game play between reminders.
                engine = None;
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        let result = (|| {
            if !control.current(command.ticket) {
                return Err(CANCELLED.into());
            }
            if engine.is_none() {
                engine = Some(KokoroEngine::open(&model_dir)?);
            }
            if !control.current(command.ticket) {
                return Err(CANCELLED.into());
            }
            let progress_control = control.clone();
            let ticket = command.ticket;
            let audio = engine
                .as_ref()
                .unwrap()
                .synthesize(&command.request, move || progress_control.current(ticket))?;
            if !control.current(ticket) {
                return Err(CANCELLED.into());
            }
            player::play(audio, command.request.volume, control.clone(), ticket)
        })();
        let _ = command.reply.send(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> SpeechRequest {
        SpeechRequest {
            text: "小龙即将刷新".into(),
            voice_id: None,
            volume: 80,
            rate: 0,
        }
    }

    #[test]
    fn creation_and_cancellation_are_lazy_without_model_or_audio_device() {
        let directory = tempfile::tempdir().unwrap();
        let output = LocalSpeechOutput::new(directory.path().join("not-installed")).unwrap();
        assert!(!output.is_closed());
        assert!(!output.voices().unwrap().is_empty());
        let ticket = output.generation();
        output.cancel().unwrap();
        assert_eq!(
            output.speak_if_current(request(), ticket).unwrap_err(),
            CANCELLED
        );
        assert!(output.speak(request()).unwrap_err().contains("尚未安装"));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        output.close().unwrap();
        assert!(output.is_closed());
        output.close().unwrap();
        assert_eq!(output.speak(request()).unwrap_err(), CANCELLED);
    }

    #[test]
    fn cancellation_blocks_until_the_existing_output_stream_has_released() {
        let control = Arc::new(Control::default());
        let lease = control.begin_playback(0).unwrap();
        let cancelling = control.clone();
        let (done, result) = mpsc::channel();
        let work = std::thread::spawn(move || {
            done.send(cancelling.cancel()).unwrap();
        });
        while control.current(0) {
            std::thread::yield_now();
        }
        assert!(result.try_recv().is_err());
        drop(lease);
        result
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        work.join().unwrap();
        assert!(control.begin_playback(0).is_err());
    }

    #[test]
    fn cancelled_inference_cannot_acquire_playback_later() {
        let control = Arc::new(Control::default());
        let old_ticket = control.epoch.load(Ordering::SeqCst);
        control.cancel().unwrap();
        assert!(control.begin_playback(old_ticket).is_err());
        assert!(control
            .begin_playback(control.epoch.load(Ordering::SeqCst))
            .is_ok());
    }

    #[test]
    fn old_request_timeout_does_not_cancel_a_new_generation() {
        let control = Arc::new(Control::default());
        control.cancel().unwrap();
        let new_ticket = control.epoch.load(Ordering::SeqCst);
        let current_output = control.begin_playback(new_ticket).unwrap();
        control.cancel_if_current(0).unwrap();
        assert!(control.current(new_ticket));
        drop(current_output);
    }

    /// Opens the real default output device; run only when audible output is
    /// intended. The model stays in the caller-provided validation directory.
    #[test]
    #[ignore = "requires KOTONE_KOKORO_TEST_MODEL and an audio output device"]
    fn real_device_playback_cancel_and_close() {
        let model_dir = std::env::var_os("KOTONE_KOKORO_TEST_MODEL")
            .expect("set KOTONE_KOKORO_TEST_MODEL to an extracted official model");
        let output = Arc::new(LocalSpeechOutput::new(model_dir.into()).unwrap());
        let speaking = output.clone();
        let playback = std::thread::spawn(move || {
            speaking.speak(SpeechRequest {
                text: "收到！小龙快刷新啦，记得提前占好位置。别急，稳住这波！".into(),
                volume: 15,
                ..request()
            })
        });
        let deadline = Instant::now() + Duration::from_secs(30);
        while !output.control.rendered_audio.load(Ordering::SeqCst)
            && !playback.is_finished()
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        let rendered_audio = output.control.rendered_audio.load(Ordering::SeqCst);
        if rendered_audio {
            // Let the opened device consume several callback buffers before
            // testing the stop barrier. This is not a microphone/loopback test.
            std::thread::sleep(Duration::from_millis(100));
        }
        let cancel_started = Instant::now();
        let cancelled = output.cancel();
        let cancel_elapsed = cancel_started.elapsed();
        let released_after_cancel = !*output.control.playing.lock().unwrap();
        let close_started = Instant::now();
        let closed = output.close();
        let close_elapsed = close_started.elapsed();
        let playback_result = playback.join().unwrap();
        println!(
            "real_device: rendered_audio={rendered_audio}, cancel={cancel_elapsed:?}, close={close_elapsed:?}, playback={playback_result:?}"
        );
        assert!(rendered_audio, "no nonzero PCM reached the output callback");
        cancelled.unwrap();
        assert!(released_after_cancel);
        closed.unwrap();
        assert!(output.is_closed());
        assert!(*output.control.worker_finished.lock().unwrap());
        assert_eq!(playback_result.unwrap_err(), CANCELLED);
    }
}
