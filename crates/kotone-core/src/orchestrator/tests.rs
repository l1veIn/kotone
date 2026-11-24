use super::*;

struct NoopPorts;

impl AudioBackend for NoopPorts {
    fn start(&self, _: &str) -> Result<crate::audio::AudioHandle, String> {
        Err("audio must not open in an admission test".into())
    }
}

impl FocusBackend for NoopPorts {
    fn foreground_window(&self) -> Option<TargetWindow> {
        None
    }
    fn restore(&self, _: TargetWindow) -> bool {
        false
    }
}

impl Emitter for NoopPorts {
    fn emit(&self, _: &str, _: serde_json::Value) {}
}

fn admission_test_orchestrator() -> Arc<Orchestrator> {
    let mut orch = Orchestrator::new(
        Arc::new(RwLock::new(Settings::default())),
        Arc::new(EngineRegistry::new()),
        Arc::new(NoopPorts),
        Arc::new(crate::inject::StubInjector),
        Arc::new(NoopPorts),
        Arc::new(NoopPorts),
    );
    orch.release_grace = Duration::from_millis(1);
    orch.into_arc()
}

#[tokio::test]
async fn automatic_end_rechecks_generation_after_waiting_for_state_lock() {
    let orch = admission_test_orchestrator();
    {
        let mut inner = orch.inner.lock().unwrap();
        inner.gen = 4;
        inner.state = OrchestratorState::Listening;
    }
    let guard = orch.op.lock().await;
    let old = orch.clone();
    let ending = tokio::spawn(async move { old.end_inner(Some(4), None).await });
    tokio::task::yield_now().await;
    assert!(!ending.is_finished());
    // Model cancel + a new capture winning the lock before an old VAD/release callback.
    {
        let mut inner = orch.inner.lock().unwrap();
        inner.gen = 6;
        inner.purpose = SessionPurpose::Assistant;
    }
    drop(guard);
    ending.await.unwrap().unwrap();
    assert_eq!(orch.state(), OrchestratorState::Listening);
    assert_eq!(orch.inner.lock().unwrap().gen, 6);
}

#[tokio::test]
async fn queued_audio_error_cannot_abort_replacement_capture() {
    let orch = admission_test_orchestrator();
    let lock = orch.op.lock().await;
    {
        let mut inner = orch.inner.lock().unwrap();
        inner.gen = 4;
        inner.state = OrchestratorState::Listening;
    }
    let old = orch.clone();
    let aborting = tokio::spawn(async move {
        old.abort_with_code("录音已中断", "AUDIO_DEVICE_ERROR", Some(4))
            .await;
    });
    tokio::task::yield_now().await;
    assert!(!aborting.is_finished());
    {
        let mut inner = orch.inner.lock().unwrap();
        inner.gen = 6;
        inner.release_grace_task = Some(tokio::spawn(std::future::pending::<()>()));
    }
    drop(lock);
    aborting.await.unwrap();
    let mut inner = orch.inner.lock().unwrap();
    assert_eq!(inner.state, OrchestratorState::Listening);
    assert_eq!(inner.gen, 6);
    inner.release_grace_task.take().unwrap().abort();
}

#[tokio::test]
async fn expired_release_timer_remains_abortable_while_waiting_for_state_lock() {
    let orch = admission_test_orchestrator();
    orch.inner.lock().unwrap().state = OrchestratorState::Listening;
    let guard = orch.op.lock().await;
    orch.schedule_release_end(0, orch.hotkey_input_ticket());
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(orch.inner.lock().unwrap().release_grace_task.is_some());
    orch.abort_release_grace();
    drop(guard);
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(orch.state(), OrchestratorState::Listening);
}

#[tokio::test]
async fn queued_old_hotkey_actions_cannot_end_cancel_or_confirm_a_new_session() {
    for action in 0..3 {
        let orch = admission_test_orchestrator();
        {
            let mut inner = orch.inner.lock().unwrap();
            inner.gen = 4;
            inner.state = OrchestratorState::Preview;
        }
        let ticket = orch.hotkey_input_ticket();
        let guard = orch.op.lock().await;
        let old = orch.clone();
        let task = tokio::spawn(async move {
            match action {
                0 => old.cancel_inner(Some(ticket), Some(4)).await,
                1 => {
                    old.end_inner(Some(4), Some(ticket)).await.unwrap();
                }
                _ => {
                    old.confirm_send_inner(Some(ticket), Some(4)).await.unwrap();
                }
            }
        });
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        orch.invalidate_hotkey_inputs();
        {
            let mut inner = orch.inner.lock().unwrap();
            inner.gen = 6;
            inner.state = OrchestratorState::Listening;
            inner.purpose = SessionPurpose::Assistant;
        }
        drop(guard);
        task.await.unwrap();
        assert_eq!(orch.state(), OrchestratorState::Listening);
        assert_eq!(orch.inner.lock().unwrap().gen, 6);
    }
}

#[test]
fn input_configuration_guard_rejects_all_active_phases_and_unfinished_send() {
    let orch = admission_test_orchestrator();
    for state in [
        OrchestratorState::Listening,
        OrchestratorState::Transcribing,
        OrchestratorState::Processing,
        OrchestratorState::Preview,
        OrchestratorState::Sending,
    ] {
        orch.inner.lock().unwrap().state = state;
        assert!(orch.try_input_configuration_guard().is_err());
    }
    for state in [
        OrchestratorState::Idle,
        OrchestratorState::Success,
        OrchestratorState::Error,
    ] {
        orch.inner.lock().unwrap().state = state;
        let guard = orch.try_input_configuration_guard().unwrap();
        assert!(orch.op.try_lock().is_err());
        drop(guard);
    }
    let (operation, _completion) = SendOperation::new();
    orch.inner.lock().unwrap().send_operation = Some(operation);
    assert!(orch.try_input_configuration_guard().is_err());
}

/// 热词端到端链路的 core 段：profile.hotwords 原样进 SessionConfig
/// （stt 段见 online_transducer 的 format_hotwords 测试）
#[test]
fn session_config_carries_profile_hotwords() {
    let settings = Settings::default();
    let profile = GameProfile::builtin_lol();
    let cfg = build_session_config(&settings, "sherpa-onnx-x-asr-zh-en", &profile);
    assert_eq!(cfg.hotwords, profile.hotwords);
    assert!(cfg.hotwords.contains(&"打野".to_string()));
    assert_eq!(cfg.language, "zh");
    assert_eq!(
        cfg.hotwords_score, settings.hotwords_score,
        "热词权重原样携带"
    );
}

#[test]
fn session_config_carries_custom_hotwords_score() {
    let settings = Settings {
        hotwords_score: 1.5,
        ..Settings::default()
    };
    let generic = GameProfile::builtin_generic();
    let cfg = build_session_config(&settings, "sherpa-onnx-x-asr-zh-en", &generic);
    assert_eq!(cfg.hotwords_score, 1.5);
}

#[test]
fn session_config_merges_model_schema_values() {
    let mut settings = Settings {
        active_model_id: "sense-voice-zh-en-ja-ko-yue-2024-07-17".into(),
        ..Settings::default()
    };
    settings.model_configs.insert(
        "sense-voice-zh-en-ja-ko-yue-2024-07-17".into(),
        serde_json::json!({ "language": "yue" }),
    );
    let cfg = build_session_config(
        &settings,
        "sherpa-onnx-sensevoice",
        &GameProfile::builtin_generic(),
    );
    assert_eq!(cfg.options["language"], "yue");
}

#[test]
fn session_config_options_fallback_null() {
    let mut settings = Settings::default();
    settings.engine_options["sherpa-onnx-x-asr-zh-en"]["provider"] = serde_json::json!("cpu");
    let generic = GameProfile::builtin_generic();
    let cfg = build_session_config(&settings, "sherpa-onnx-x-asr-zh-en", &generic);
    assert_eq!(cfg.options["provider"], "cpu");
    assert!(cfg.hotwords.is_empty(), "generic 无热词");
    // 未配置的引擎 → 空对象（可再合并模型 configSchema）
    let cfg2 = build_session_config(&settings, "mock-stream", &generic);
    assert!(cfg2.options.as_object().is_some_and(|o| o.is_empty()));
}
