//! Windows SAPI 本机播报。所有 COM 对象只存活在同一个工作线程；调用端只持有通道。

use kotone_core::speech::{SpeechOutput, SpeechRequest, SpeechVoice};

#[cfg(windows)]
mod imp {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::{Duration, Instant};
    use windows::core::{Interface, PCWSTR, PWSTR};
    use windows::Win32::Media::Speech::{
        ISpObjectToken, ISpObjectTokenCategory, ISpVoice, SpObjectTokenCategory, SpVoice,
        SPCAT_VOICES, SPF_ASYNC, SPF_IS_NOT_XML, SPF_PURGEBEFORESPEAK, SPVOICESTATUS,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    };

    type Reply<T> = mpsc::SyncSender<Result<T, String>>;
    type ErrorHandler = Arc<dyn Fn(String) + Send + Sync>;
    enum Command {
        Speak {
            epoch: u64,
            request: SpeechRequest,
            reply: Reply<()>,
        },
    }
    struct Control {
        epoch: AtomicU64,
        stopped: AtomicBool,
    }

    pub struct WindowsSpeechOutput {
        commands: mpsc::SyncSender<Command>,
        cancellations: mpsc::SyncSender<Reply<()>>,
        control: Arc<Control>,
        voices: Vec<SpeechVoice>,
    }

    impl WindowsSpeechOutput {
        pub fn new() -> Result<Self, String> {
            Self::with_error_handler(Arc::new(|error| {
                kotone_core::log::log(&format!("语音播报失败: {error}"))
            }))
        }

        pub fn with_error_handler(on_error: ErrorHandler) -> Result<Self, String> {
            let (commands, rx) = mpsc::sync_channel(8);
            let (cancellations, cancel_rx) = mpsc::sync_channel(1);
            let (ready, boot) = mpsc::sync_channel(1);
            let control = Arc::new(Control {
                epoch: AtomicU64::new(0),
                stopped: AtomicBool::new(false),
            });
            let worker_control = control.clone();
            std::thread::Builder::new()
                .name("kotone-speech".into())
                .spawn(move || worker(rx, cancel_rx, worker_control, ready, on_error))
                .map_err(|error| format!("启动语音播报线程失败: {error}"))?;
            match boot.recv_timeout(Duration::from_secs(5)) {
                Ok(Ok(voices)) => Ok(Self {
                    commands,
                    cancellations,
                    control,
                    voices,
                }),
                result => {
                    control.stopped.store(true, Ordering::SeqCst);
                    Err(match result {
                        Ok(Err(error)) => error,
                        _ => "初始化 Windows 语音服务超时".into(),
                    })
                }
            }
        }

        fn response<T>(
            rx: mpsc::Receiver<Result<T, String>>,
            timeout: Duration,
        ) -> Result<T, String> {
            rx.recv_timeout(timeout)
                .map_err(|_| "Windows 语音服务未及时响应".to_string())?
        }
    }

    impl SpeechOutput for WindowsSpeechOutput {
        fn voices(&self) -> Result<Vec<SpeechVoice>, String> {
            Ok(self.voices.clone())
        }

        fn generation(&self) -> u64 {
            self.control.epoch.load(Ordering::SeqCst)
        }

        fn speak_if_current(&self, request: SpeechRequest, epoch: u64) -> Result<(), String> {
            request.validate()?;
            if epoch != self.generation() {
                return Err("播报已取消".into());
            }
            let (reply, rx) = mpsc::sync_channel(1);
            self.commands
                .try_send(Command::Speak {
                    epoch,
                    request,
                    reply,
                })
                .map_err(|_| "播报队列已满或语音服务已停止".to_string())?;
            match rx.recv_timeout(Duration::from_secs(60)) {
                Ok(result) => result,
                Err(_) => {
                    self.control.epoch.fetch_add(1, Ordering::SeqCst);
                    Err("Windows 语音服务未及时响应，播报已取消".into())
                }
            }
        }

        fn cancel(&self) -> Result<(), String> {
            self.control.epoch.fetch_add(1, Ordering::SeqCst);
            let (reply, rx) = mpsc::sync_channel(1);
            self.cancellations
                .try_send(reply)
                .map_err(|_| "语音取消正在处理或语音服务已停止".to_string())?;
            Self::response(rx, Duration::from_secs(2))
        }
    }

    impl Drop for WindowsSpeechOutput {
        fn drop(&mut self) {
            self.control.stopped.store(true, Ordering::SeqCst);
        }
    }

    struct ComApartment;
    impl ComApartment {
        fn initialize() -> Result<Self, String> {
            unsafe {
                CoInitializeEx(None, COINIT_MULTITHREADED)
                    .ok()
                    .map_err(|e| e.to_string())?;
            }
            Ok(Self)
        }
    }
    impl Drop for ComApartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }

    struct VoiceEngine {
        voice: ISpVoice,
        default_voice: ISpObjectToken,
    }
    impl VoiceEngine {
        fn new() -> Result<Self, String> {
            unsafe {
                let voice: ISpVoice = CoCreateInstance(&SpVoice, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| format!("Windows 语音服务不可用: {e}"))?;
                let default_voice = voice
                    .GetVoice()
                    .map_err(|e| format!("未找到 Windows 默认语音，请在系统设置安装语音包: {e}"))?;
                Ok(Self {
                    voice,
                    default_voice,
                })
            }
        }
        fn tokens(&self) -> Result<Vec<(SpeechVoice, ISpObjectToken)>, String> {
            unsafe {
                let category: ISpObjectTokenCategory =
                    CoCreateInstance(&SpObjectTokenCategory, None, CLSCTX_INPROC_SERVER)
                        .map_err(|e| e.to_string())?;
                category
                    .SetId(SPCAT_VOICES, false)
                    .map_err(|e| e.to_string())?;
                let tokens = category
                    .EnumTokens(PCWSTR::null(), PCWSTR::null())
                    .map_err(|e| e.to_string())?;
                let mut count = 0;
                tokens.GetCount(&mut count).map_err(|e| e.to_string())?;
                let mut result = Vec::new();
                for index in 0..count {
                    let token = tokens.Item(index).map_err(|e| e.to_string())?;
                    let id = take_com_string(token.GetId().map_err(|e| e.to_string())?);
                    let name = token
                        .GetStringValue(PCWSTR::null())
                        .map(|value| take_com_string(value))
                        .unwrap_or_else(|_| id.clone());
                    result.push((SpeechVoice { id, name }, token));
                }
                Ok(result)
            }
        }
        fn purge(&self) -> Result<(), String> {
            unsafe {
                self.voice
                    .Speak(
                        PCWSTR::null(),
                        (SPF_ASYNC.0 | SPF_PURGEBEFORESPEAK.0 | SPF_IS_NOT_XML.0) as u32,
                        None,
                    )
                    .map_err(|e| e.to_string())
            }
        }
        fn speak(&self, request: SpeechRequest) -> Result<(), String> {
            let token = match request
                .voice_id
                .as_deref()
                .filter(|id| !id.trim().is_empty())
            {
                Some(id) => self
                    .tokens()?
                    .into_iter()
                    .find(|(voice, _)| voice.id == id)
                    .map(|(_, token)| token)
                    .ok_or_else(|| "所选 Windows 语音已不可用，请重新选择".to_string())?,
                None => self.default_voice.clone(),
            };
            let text: Vec<u16> = request
                .text
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            unsafe {
                self.voice.SetVoice(&token).map_err(|e| e.to_string())?;
                self.voice
                    .SetRate(request.rate)
                    .map_err(|e| e.to_string())?;
                self.voice
                    .SetVolume(request.volume)
                    .map_err(|e| e.to_string())?;
                // 明确按纯文本处理，LLM/用户文本中的 XML 不能触发文件或标记解析。
                self.voice
                    .Speak(
                        PCWSTR(text.as_ptr()),
                        (SPF_ASYNC.0 | SPF_IS_NOT_XML.0) as u32,
                        None,
                    )
                    .map_err(|e| e.to_string())
            }
        }
    }

    unsafe fn take_com_string(value: PWSTR) -> String {
        let result = unsafe { value.to_string() }.unwrap_or_default();
        unsafe {
            CoTaskMemFree(Some(value.0.cast()));
        }
        result
    }

    fn worker(
        rx: mpsc::Receiver<Command>,
        cancel_rx: mpsc::Receiver<Reply<()>>,
        control: Arc<Control>,
        ready: Reply<Vec<SpeechVoice>>,
        on_error: ErrorHandler,
    ) {
        let _apartment = match ComApartment::initialize() {
            Ok(value) => value,
            Err(error) => {
                let _ = ready.send(Err(error));
                return;
            }
        };
        let engine = match VoiceEngine::new() {
            Ok(value) => value,
            Err(error) => {
                let _ = ready.send(Err(error));
                return;
            }
        };
        let voices = match engine.tokens() {
            Ok(tokens) => tokens.into_iter().map(|(voice, _)| voice).collect(),
            Err(error) => {
                let _ = ready.send(Err(error));
                return;
            }
        };
        let _ = ready.send(Ok(voices));
        while !control.stopped.load(Ordering::SeqCst) {
            for reply in cancel_rx.try_iter() {
                let _ = reply.send(engine.purge());
            }
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(Command::Speak {
                    epoch: ticket,
                    request,
                    reply,
                }) => {
                    let result = speak_and_wait(&engine, request, ticket, &control, &cancel_rx);
                    if let Err(error) = &result {
                        if error != "播报已取消" {
                            on_error(error.clone());
                        }
                    }
                    let _ = reply.send(result);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let _ = engine.purge();
    }

    fn speak_and_wait(
        engine: &VoiceEngine,
        request: SpeechRequest,
        ticket: u64,
        control: &Control,
        cancel_rx: &mpsc::Receiver<Reply<()>>,
    ) -> Result<(), String> {
        if ticket != control.epoch.load(Ordering::SeqCst) {
            return Err("播报已取消".into());
        }
        engine.speak(request)?;
        let started = Instant::now();
        loop {
            if ticket != control.epoch.load(Ordering::SeqCst)
                || control.stopped.load(Ordering::SeqCst)
            {
                let result = engine.purge();
                for reply in cancel_rx.try_iter() {
                    let _ = reply.send(result.clone());
                }
                result?;
                return Err("播报已取消".into());
            }
            if started.elapsed() > Duration::from_secs(45) {
                engine.purge()?;
                return Err("语音播报超过 45 秒，已停止".into());
            }
            // bindings 的 Result 包装会把 S_FALSE（仍在播放）也映射为 Ok；保留 HRESULT。
            let result =
                unsafe { (engine.voice.vtable().WaitUntilDone)(engine.voice.as_raw(), 50) };
            if result.is_err() {
                let _ = engine.purge();
                return Err(result.to_string());
            }
            if result.0 == 0 {
                let mut status = SPVOICESTATUS::default();
                unsafe { engine.voice.GetStatus(&mut status, std::ptr::null_mut()) }
                    .map_err(|e| e.to_string())?;
                return status.hrLastResult.ok().map_err(|e| e.to_string());
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn cancellation_invalidates_a_ticket_before_delayed_submission() {
            // Exercise queue admission without initializing COM or producing audio.
            let (commands, queued) = mpsc::sync_channel(8);
            let (cancellations, cancelled) = mpsc::sync_channel::<Reply<()>>(1);
            let output = WindowsSpeechOutput {
                commands,
                cancellations,
                control: Arc::new(Control {
                    epoch: AtomicU64::new(0),
                    stopped: AtomicBool::new(false),
                }),
                voices: vec![],
            };
            let ticket = output.generation();
            let purge = std::thread::spawn(move || {
                cancelled.recv().unwrap().send(Ok(())).unwrap();
            });
            output.cancel().unwrap();
            let result = output.speak_if_current(
                SpeechRequest {
                    text: "不应播报".into(),
                    voice_id: None,
                    volume: 80,
                    rate: 0,
                },
                ticket,
            );
            purge.join().unwrap();
            assert!(result.unwrap_err().contains("取消"));
            assert!(matches!(queued.try_recv(), Err(mpsc::TryRecvError::Empty)));
        }

        #[test]
        #[ignore = "manual Windows voice enumeration; no audio"]
        fn enumerate_installed_voices() {
            let output = WindowsSpeechOutput::new().unwrap();
            let voices = output.voices().unwrap();
            assert!(!voices.is_empty());
            assert!(voices
                .iter()
                .all(|voice| !voice.id.is_empty() && !voice.name.is_empty()));
        }
        #[test]
        #[ignore = "manual Windows audio playback; speaks aloud"]
        fn speaks_and_cancels_plain_text() {
            let output = WindowsSpeechOutput::new().unwrap();
            output
                .speak(SpeechRequest {
                    text: "Kotone 语音提醒已准备就绪。".into(),
                    voice_id: None,
                    volume: 50,
                    rate: 0,
                })
                .unwrap();
            output.cancel().unwrap();
        }
    }
}

#[cfg(windows)]
pub use imp::WindowsSpeechOutput;

#[cfg(not(windows))]
pub struct WindowsSpeechOutput;
#[cfg(not(windows))]
impl WindowsSpeechOutput {
    pub fn new() -> Result<Self, String> {
        Err("当前平台暂不支持 Windows 语音播报".into())
    }
}
#[cfg(not(windows))]
impl SpeechOutput for WindowsSpeechOutput {
    fn voices(&self) -> Result<Vec<SpeechVoice>, String> {
        Err("当前平台暂不支持 Windows 语音播报".into())
    }
    fn generation(&self) -> u64 {
        0
    }
    fn speak_if_current(&self, _: SpeechRequest, _: u64) -> Result<(), String> {
        Err("当前平台暂不支持 Windows 语音播报".into())
    }
    fn cancel(&self) -> Result<(), String> {
        Ok(())
    }
}
