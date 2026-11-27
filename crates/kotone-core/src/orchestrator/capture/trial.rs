//! A page-owned microphone session. Only its caller receives the transcript.

use super::*;
use crate::cancel::CancellationToken;

pub(in crate::orchestrator) struct VoiceTrial {
    id: String,
    stop: Option<oneshot::Sender<()>>,
    result: Option<oneshot::Receiver<Result<String, String>>>,
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl VoiceTrial {
    pub(in crate::orchestrator) fn running(&self) -> bool {
        !self.task.is_finished()
    }

    pub(in crate::orchestrator) fn cancel(&self) {
        self.cancel.cancel();
    }
}

impl Orchestrator {
    pub async fn begin_voice_trial(&self) -> Result<String, String> {
        let _op = self.op.lock().await;
        if !self.runtime_inputs_enabled.load(Ordering::SeqCst) {
            return Err("请先启动语音识别".into());
        }
        let settings = self.settings.read().unwrap().clone();
        if !settings.plugin_enabled("voice-input", true) {
            return Err("请先启用语音输入".into());
        }
        let clear_terminal = {
            let mut inner = self.inner.lock().unwrap();
            if !matches!(
                inner.state,
                OrchestratorState::Idle | OrchestratorState::Success | OrchestratorState::Error
            ) || inner.send_is_running()
                || inner.trial.as_ref().is_some_and(VoiceTrial::running)
            {
                return Err("已有语音会话正在进行".into());
            }
            inner.state != OrchestratorState::Idle
        };
        if clear_terminal {
            self.cancel_locked();
        }
        let SttStart { session, .. } = self.start_stt_session(&settings).map_err(|e| e.message)?;
        let (session, completion, handle) = self
            .open_microphone(&settings, session)
            .map_err(|e| e.message)?;
        let (stop_tx, stop_rx) = oneshot::channel();
        let (result_tx, result_rx) = oneshot::channel();
        let cancel = CancellationToken::default();
        let worker_cancel = cancel.clone();
        let timeout = self.finalize_timeout;
        let task = tokio::spawn(async move {
            let result =
                record_trial(session, handle, completion, stop_rx, worker_cancel, timeout).await;
            let _ = result_tx.send(result);
        });
        let id = NEXT_PROCESS_SESSION_ID
            .fetch_add(1, Ordering::Relaxed)
            .to_string();
        self.inner.lock().unwrap().trial = Some(VoiceTrial {
            id: id.clone(),
            stop: Some(stop_tx),
            result: Some(result_rx),
            cancel,
            task,
        });
        Ok(id)
    }

    pub async fn finish_voice_trial(&self, id: &str) -> Result<String, String> {
        let (result, cancel) = {
            let _op = self.op.lock().await;
            let mut inner = self.inner.lock().unwrap();
            let trial = inner
                .trial
                .as_mut()
                .filter(|t| t.id == id)
                .ok_or("试说已结束")?;
            let result = trial.result.take().ok_or("试说已结束")?;
            if let Some(stop) = trial.stop.take() {
                let _ = stop.send(());
            }
            (result, trial.cancel.clone())
        };
        let transcript = result
            .await
            .map_err(|_| "试说已结束".to_string())
            .and_then(|r| r);
        let _op = self.op.lock().await;
        let mut inner = self.inner.lock().unwrap();
        if !inner.trial.as_ref().is_some_and(|t| t.id == id) || cancel.is_cancelled() {
            return Err("试说已取消".into());
        }
        inner.trial = None;
        transcript
    }

    pub async fn cancel_voice_trial(&self, id: &str) {
        let _op = self.op.lock().await;
        let trial = {
            let mut inner = self.inner.lock().unwrap();
            if inner.trial.as_ref().is_some_and(|t| t.id == id) {
                inner.trial.take()
            } else {
                None
            }
        };
        if let Some(trial) = trial {
            trial.cancel();
            let _ = trial.task.await;
        }
    }
}

async fn record_trial(
    mut session: Box<dyn SttSession>,
    mut handle: AudioHandle,
    completion: CaptureCompletion,
    mut stop: oneshot::Receiver<()>,
    cancel: CancellationToken,
    finalize_timeout: Duration,
) -> Result<String, String> {
    let mut pcm = handle.pcm_rx.take().expect("audio handle PCM channel");
    let mut errors = handle.error_rx.take();
    let guard = CaptureGuard {
        audio: Some(handle),
        _completion: completion,
    };
    let deadline = tokio::time::sleep(Duration::from_secs(60));
    tokio::pin!(deadline);
    let capture = loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break Err("试说已取消".to_string()),
            _ = &mut deadline => break Err("试说已超时，请重新开始".to_string()),
            error = async { errors.as_mut().unwrap().recv().await }, if errors.is_some() => {
                if let Some(error) = error { break Err(error); }
                errors = None;
            }
            end = &mut stop => break end.map_err(|_| "试说已结束".to_string()),
            chunk = pcm.recv() => match chunk {
                Some(chunk) => if let Err(error) = session.push_audio(&chunk) { break Err(error); },
                None => break Ok(()),
            }
        }
    };
    drop(guard);
    if let Err(error) = capture {
        session.cancel();
        return Err(error);
    }
    while let Ok(chunk) = pcm.try_recv() {
        session.push_audio(&chunk)?;
    }
    let task = tokio::task::spawn_blocking(move || session.finalize());
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err("试说已取消".into()),
        result = tokio::time::timeout(finalize_timeout, task) => {
            let transcript = result.map_err(|_| "识别超时")?.map_err(|_| "识别线程异常")??;
            Ok(transcript.text)
        }
    }
}
