//! Serializes the desktop's background prewarm, session construction and cache eviction.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, Notify};
use tokio::task::{AbortHandle, JoinHandle};

use crate::cancel::CancellationToken;
use crate::stt::{EngineRegistry, SessionConfig, SttEngine, SttEvent, SttSession, Transcript};

static NEXT_ACTIVATION: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
pub(crate) struct ModelCache(Mutex<Option<(u64, String)>>);

struct Usage {
    idle: bool,
    users: usize,
    ticket: u64,
    timer: Option<AbortHandle>,
}

impl Usage {
    fn invalidate_timer(&mut self) {
        self.ticket += 1;
        if let Some(timer) = self.timer.take() {
            timer.abort();
        }
    }
}

pub(crate) struct RuntimeModel {
    activation: u64,
    engine_id: String,
    engines: Arc<EngineRegistry>,
    cache: Arc<ModelCache>,
    idle_timeout: Option<Duration>,
    retired: AtomicBool,
    released: AtomicBool,
    release_done: Notify,
    usage: Mutex<Usage>,
}

impl RuntimeModel {
    pub(crate) fn new(
        engine_id: String,
        engines: Arc<EngineRegistry>,
        cache: Arc<ModelCache>,
        idle_timeout: Option<Duration>,
    ) -> Arc<Self> {
        Arc::new(Self {
            activation: NEXT_ACTIVATION.fetch_add(1, Ordering::Relaxed),
            engine_id,
            engines,
            cache,
            idle_timeout,
            retired: AtomicBool::new(false),
            released: AtomicBool::new(false),
            release_done: Notify::new(),
            usage: Mutex::new(Usage {
                idle: true,
                users: 0,
                ticket: 0,
                timer: None,
            }),
        })
    }

    pub(crate) fn set_idle(self: &Arc<Self>, idle: bool) {
        let mut usage = self.usage.lock().unwrap();
        if usage.idle == idle {
            return;
        }
        usage.idle = idle;
        usage.invalidate_timer();
        self.schedule_idle_release(&mut usage);
    }

    fn pin(self: &Arc<Self>) -> ModelUse {
        let mut usage = self.usage.lock().unwrap();
        usage.invalidate_timer();
        usage.users += 1;
        ModelUse(self.clone())
    }

    fn finish_use(self: &Arc<Self>) {
        let mut usage = self.usage.lock().unwrap();
        usage.users -= 1;
        if usage.users != 0 {
            return;
        }
        if self.retired.load(Ordering::Acquire) {
            drop(usage);
            self.release_retired();
        } else {
            self.schedule_idle_release(&mut usage);
        }
    }

    fn schedule_idle_release(self: &Arc<Self>, usage: &mut Usage) {
        let Some(timeout) = self.idle_timeout else {
            return;
        };
        if !usage.idle || usage.users != 0 || self.retired.load(Ordering::Acquire) {
            return;
        }
        let ticket = usage.ticket;
        let model = Arc::downgrade(self);
        usage.timer = Some(
            tokio::spawn(async move {
                tokio::time::sleep(timeout).await;
                if let Some(model) = model.upgrade() {
                    let _ = tokio::task::spawn_blocking(move || {
                        let mut resident = model.cache.0.lock().unwrap();
                        {
                            let mut usage = model.usage.lock().unwrap();
                            if usage.ticket != ticket
                                || !usage.idle
                                || usage.users != 0
                                || model.retired.load(Ordering::Acquire)
                            {
                                return;
                            }
                            usage.timer = None;
                        }
                        if model.release_if_owned(&mut resident) {
                            crate::log::log(&format!(
                                "runtime model idle unloaded: {}",
                                model.engine_id
                            ));
                        }
                    })
                    .await;
                }
            })
            .abort_handle(),
        );
    }

    pub(crate) fn retire(self: &Arc<Self>) {
        let unused = {
            let mut usage = self.usage.lock().unwrap();
            if self.retired.swap(true, Ordering::AcqRel) {
                return;
            }
            usage.invalidate_timer();
            usage.users == 0
        };
        if unused {
            self.release_retired();
        }
    }

    pub(crate) async fn wait_released(&self) {
        let done = self.release_done.notified();
        tokio::pin!(done);
        done.as_mut().enable();
        if !self.released.load(Ordering::Acquire) {
            done.await;
        }
    }

    fn release_retired(self: &Arc<Self>) {
        let model = self.clone();
        tokio::spawn(async move {
            let worker = model.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let mut resident = worker.cache.0.lock().unwrap_or_else(|err| err.into_inner());
                worker.release_if_owned(&mut resident);
            })
            .await;
            model.released.store(true, Ordering::Release);
            model.release_done.notify_waiters();
        });
    }

    fn release_if_owned(&self, resident: &mut Option<(u64, String)>) -> bool {
        if !resident
            .as_ref()
            .is_some_and(|(activation, _)| *activation == self.activation)
        {
            return false;
        }
        if let Some(engine) = self.engines.get(&self.engine_id) {
            engine.unload();
        }
        *resident = None;
        true
    }

    fn prepare<T>(
        &self,
        cancelled: Option<&CancellationToken>,
        work: impl FnOnce(&dyn SttEngine, bool) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut resident = self.cache.0.lock().unwrap();
        if self.retired.load(Ordering::Acquire)
            || cancelled.is_some_and(CancellationToken::is_cancelled)
        {
            return Err("录音或模型加载已经取消".into());
        }
        let cold = resident
            .as_ref()
            .map_or(true, |(activation, _)| *activation != self.activation);
        if cold {
            if let Some((_, previous)) = resident.take() {
                if let Some(engine) = self.engines.get(&previous) {
                    engine.unload();
                }
            }
        }
        let engine = self
            .engines
            .get(&self.engine_id)
            .ok_or_else(|| format!("未注册的 STT 引擎: {}", self.engine_id))?;
        let loading = cold && engine.capabilities().offline;
        if loading {
            crate::log::log(&format!("runtime model load started: {}", self.engine_id));
        }
        let result = work(engine, cold);
        if cold && result.is_ok() {
            *resident = Some((self.activation, self.engine_id.clone()));
            if loading {
                crate::log::log(&format!("runtime model load completed: {}", self.engine_id));
            }
        } else if loading {
            // Session initialization can fail after creating a shared recognizer.
            // Such a recognizer has no resident owner yet, so discard it here.
            engine.unload();
        }
        result
    }

    pub(crate) fn prewarm(
        self: Arc<Self>,
        config: SessionConfig,
    ) -> JoinHandle<Result<(), String>> {
        let pin = self.pin();
        tokio::task::spawn_blocking(move || {
            let _pin = pin;
            self.prepare(None, |engine, cold| {
                if cold && engine.capabilities().offline {
                    engine.warmup(&config)?;
                }
                Ok(())
            })
        })
    }

    pub(crate) fn start_session(
        self: &Arc<Self>,
        config: SessionConfig,
        events: mpsc::UnboundedSender<SttEvent>,
        cancelled: CancellationToken,
        capture_announced: tokio::sync::oneshot::Receiver<()>,
    ) -> JoinHandle<Result<Box<dyn SttSession>, String>> {
        let model = self.clone();
        let pin = self.pin();
        tokio::spawn(async move {
            capture_announced
                .await
                .map_err(|_| "录音已取消".to_string())?;
            tokio::task::spawn_blocking(move || {
                let mut session = model.prepare(Some(&cancelled), |engine, _| {
                    engine.start_session(&config, events)
                })?;
                if cancelled.is_cancelled() {
                    session.cancel();
                    return Err("录音已取消".into());
                }
                Ok(Box::new(PinnedSession { session, _pin: pin }) as Box<dyn SttSession>)
            })
            .await
            .map_err(|error| format!("识别启动线程异常：{error}"))?
        })
    }
}

struct ModelUse(Arc<RuntimeModel>);

impl Drop for ModelUse {
    fn drop(&mut self) {
        self.0.finish_use();
    }
}

// The engine must remain pinned for the full native call, even if its caller
// cancels or times out and the blocking finalizer has not returned yet.
struct PinnedSession {
    session: Box<dyn SttSession>,
    _pin: ModelUse,
}

impl SttSession for PinnedSession {
    fn push_audio(&mut self, pcm: &[f32]) -> Result<(), String> {
        self.session.push_audio(pcm)
    }

    fn finalize(self: Box<Self>) -> Result<Transcript, String> {
        let Self { session, _pin } = *self;
        session.finalize()
    }

    fn cancel(&mut self) {
        self.session.cancel();
    }
}
