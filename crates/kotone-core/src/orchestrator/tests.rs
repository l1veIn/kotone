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

// ---------- 发送频道解析（ADR-008） ----------

#[test]
fn resolve_send_strategy_uses_selected_channel() {
    let lol = GameProfile::builtin_lol();
    let sel = Some(("lol".to_string(), "all".to_string()));
    let s = resolve_send_strategy(&lol, &sel);
    assert_eq!(s.id, "all");
    assert_eq!(s.open_chat_key, "Shift+Enter");
}

#[test]
fn resolve_send_strategy_falls_back_when_profile_switched() {
    // 频道残留属于 lol，但当前 profile 已是 generic → 默认频道
    let generic = GameProfile::builtin_generic();
    let sel = Some(("lol".to_string(), "all".to_string()));
    let s = resolve_send_strategy(&generic, &sel);
    assert!(s.is_default);
    assert_eq!(s.open_chat_key, generic.open_chat_key);
    // 未选择（None）同样回落默认
    let s2 = resolve_send_strategy(&lol_clone(), &None);
    assert_eq!(s2.id, "team");
}

fn lol_clone() -> GameProfile {
    GameProfile::builtin_lol()
}

#[test]
fn wire_profile_makes_the_selected_channel_the_default() {
    let mut dota = GameProfile::builtin_dota2();
    dota.channels[0].console_command = Some("say_team".into());
    dota.channels[1].console_command = Some("say".into());
    let sel = Some(("dota2".to_string(), "all".to_string()));
    let wire = dota.for_channel(&resolve_send_strategy(&dota, &sel));
    assert_eq!(wire.open_chat_key, "Shift+Enter");
    let sent = wire.channel_strategy(None);
    assert_eq!(sent.id, "all");
    assert_eq!(sent.console_command, "say");
    // 没选频道：默认频道（队伍）
    let wire = dota.for_channel(&resolve_send_strategy(&dota, &None));
    assert_eq!(wire.open_chat_key, "Enter");
    assert_eq!(wire.channel_strategy(None).console_command, "say_team");
}

// ---------- 历史音频 VAD 裁剪（trim_speech_pcm / push_speech_range） ----------

#[test]
fn trim_speech_pcm_no_segments_returns_original() {
    let pcm = vec![0.5f32; 1000];
    let out = trim_speech_pcm(&pcm, &[]);
    assert_eq!(out, pcm, "非 VAD 模式（无区间）应原样保存");
}

#[test]
fn trim_speech_pcm_single_segment_with_prepost_roll() {
    let pcm: Vec<f32> = (0..10000).map(|i| i as f32).collect();
    // 段 [1000,2000)：前导缓冲取到 0（PRE 2400 超出开头），尾随缓冲到 4400
    let out = trim_speech_pcm(&pcm, &[(1000, 2000)]);
    assert_eq!(out, pcm[0..4400]);
    // 段 [9000,9500)：尾随缓冲 clamp 到 pcm 末尾；前导取到 6600
    let out2 = trim_speech_pcm(&pcm, &[(9000, 9500)]);
    assert_eq!(out2, pcm[6600..10000]);
    // 段 [500,1500)：前后都 clamp 到界内
    let out3 = trim_speech_pcm(&pcm, &[(500, 1500)]);
    assert_eq!(out3, pcm[0..3900]);
}

#[test]
fn trim_speech_pcm_merges_close_segments_after_roll() {
    let pcm: Vec<f32> = (0..10000).map(|i| i as f32).collect();
    // 两段间隙 2000 采样（125ms）< 缓冲覆盖（PRE+POST=4800）→ 合并成一段
    let out = trim_speech_pcm(&pcm, &[(1000, 2000), (4000, 5000)]);
    assert_eq!(out, pcm[0..7400]);
    // 长停顿（间隙 6000 采样 = 375ms > 4800）→ 断开，中间静音裁掉
    let out2 = trim_speech_pcm(&pcm, &[(1000, 2000), (8000, 9000)]);
    assert_eq!(out2.len(), 4400 + 4400);
    assert_eq!(out2[..4400], pcm[0..4400]);
    assert_eq!(out2[4400..], pcm[5600..10000]);
}

#[test]
fn trim_speech_pcm_clamps_out_of_bounds() {
    let pcm: Vec<f32> = (0..50).map(|i| i as f32).collect();
    // 短 PCM + 越界区间：全部扩展后重叠合并、clamp 到 pcm 边界，不 panic
    let out = trim_speech_pcm(&pcm, &[(40, 100), (200, 300)]);
    assert_eq!(out, pcm, "区间越界时应整体 clamp 到 pcm 长度");
}

#[test]
fn push_speech_range_merges_close_segments() {
    let mut g = HistoryDraft::speech_harness();
    // 帧 0-10 语音，停顿 100ms（1600 采样 < 300ms 合并阈值）后又语音
    g.push_speech_range(0, 4800);
    g.push_speech_range(6400, 9600); // 间隙 1600 采样 = 100ms → 合并
    assert_eq!(g.speech_segments, vec![(0, 9600)]);
    // 长停顿（> 300ms）后断开
    g.push_speech_range(19200, 24000); // 距 9600 差 9600 采样 = 600ms → 新段
    assert_eq!(g.speech_segments, vec![(0, 9600), (19200, 24000)]);
    // 无效区间忽略
    g.push_speech_range(50000, 50000);
    assert_eq!(g.speech_segments.len(), 2);
}

#[test]
fn vad_speech_seen_defaults_false() {
    let g = HistoryDraft::speech_harness();
    assert!(!g.vad_speech_seen);
    assert!(g.speech_segments.is_empty());
}
