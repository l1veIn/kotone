//! 语音输入的单元测试（从 `kotone-core` 的 `orchestrator/tests.rs` 搬来，名字和内容不变）。

use crate::records::{trim_speech_pcm, HistoryDraft};

// 发送频道解析（ADR-008）的测试随规则搬进了本体的入口：
// kotone-core/src/send_to_game/tests.rs 的 a_voice_channel_chosen_in_another_game_falls_back_to_the_default、
// the_chosen_channel_picks_the_console_command。

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

// ---------- 推送卡片和重发（ADR 043 D10） ----------
//
// 用本体的编排器接上语音输入，“发到游戏”入口换成替身（记下每次的请求、按排好的结果回），宿主换成替身（记下推送和事件）。
// 不开麦克风、不按键：发送从“空着”占住发送权开始（和重发最近一条热键同一条路）。

mod pushes_and_resend {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex, RwLock};
    use std::time::Duration;

    use kotone_core::audio::{AudioBackend, AudioHandle};
    use kotone_core::history::{HistoryMode, HistoryOutcome, HistoryRecord};
    use kotone_core::inject::{FocusBackend, InjectError, TargetWindow};
    use kotone_core::input_lock::{InputBusy, InputUser};
    use kotone_core::orchestrator::{Emitter, Orchestrator, OrchestratorState, ReadyText};
    use kotone_core::send_to_game::{
        ChannelPick, ChatFailure, ChatOutcome, ChatRequest, ChatStop, SendToGame, Speaker, StopCode,
    };
    use kotone_core::settings::Settings;
    use kotone_core::stt::EngineRegistry;
    use kotone_plugin_api::{BoxFuture, Host, Observation, Push, PushOutcome};
    use serde_json::{json, Map, Value};

    use crate::pushes::tests::outcome;
    use crate::pushes::{Event, PAGE_FAILED, PUSH_FAILED, PUSH_SENT};
    use crate::records::{DraftCell, HistoryDraft};
    use crate::send::{HistoryWrite, Resent, Route, NO_GAME_WINDOW};
    use crate::VoiceInputExt;

    struct Ports;

    impl AudioBackend for Ports {
        fn start(&self, _: &str) -> Result<AudioHandle, String> {
            Err("测试不开麦克风".into())
        }
    }

    impl FocusBackend for Ports {
        fn foreground_window(&self) -> Option<TargetWindow> {
            None
        }
        fn restore(&self, _: TargetWindow) -> bool {
            false
        }
    }

    impl Emitter for Ports {
        fn emit(&self, _: &str, _: Value) {}
    }

    type Answer = Box<dyn FnOnce() -> Result<ChatOutcome, ChatFailure> + Send>;

    /// 本体“发到游戏”入口的替身：记下每次的请求，按排好的回（没排的就当发出去了）。
    #[derive(Default)]
    struct Entry {
        asked: Mutex<Vec<ChatRequest>>,
        answers: Mutex<VecDeque<Answer>>,
    }

    impl Entry {
        fn answer(
            &self,
            answer: impl FnOnce() -> Result<ChatOutcome, ChatFailure> + Send + 'static,
        ) {
            self.answers.lock().unwrap().push_back(Box::new(answer));
        }
        fn asked(&self) -> Vec<ChatRequest> {
            self.asked.lock().unwrap().clone()
        }
    }

    impl SendToGame for Entry {
        fn send(&self, request: ChatRequest) -> Result<ChatOutcome, ChatFailure> {
            let text = request.text.clone();
            let channel = request
                .channel
                .as_ref()
                .map_or("team", |c| c.id.as_str())
                .to_string();
            self.asked.lock().unwrap().push(request);
            let answer = self.answers.lock().unwrap().pop_front();
            match answer {
                Some(answer) => answer(),
                None => {
                    let name = if channel == "all" {
                        "所有人"
                    } else {
                        "队伍"
                    };
                    Ok(outcome(("lol", "英雄联盟"), (&channel, name), &text))
                }
            }
        }
    }

    /// 宿主的替身：记下推送（同一个 key 第二次算更新）和事件。
    #[derive(Default)]
    struct TestHost {
        pushes: Mutex<Vec<Push>>,
        events: Mutex<Vec<Event>>,
    }

    impl TestHost {
        fn pushes(&self) -> Vec<Push> {
            self.pushes.lock().unwrap().clone()
        }
        fn last(&self, trigger: &str) -> Push {
            self.pushes()
                .into_iter()
                .rev()
                .find(|p| p.trigger == trigger)
                .unwrap_or_else(|| panic!("没有推送 {trigger}：{:?}", self.pushes()))
        }
    }

    impl Host for TestHost {
        fn plugin(&self) -> &str {
            "voice-input"
        }
        fn enabled(&self) -> bool {
            true
        }
        fn source_enabled(&self, _: &str) -> bool {
            true
        }
        fn settings(&self) -> Value {
            json!({})
        }
        fn data_dir(&self) -> std::path::PathBuf {
            std::env::temp_dir()
        }
        fn spawn(&self, task: BoxFuture<'static, ()>) {
            tokio::spawn(task);
        }
        fn process_ids(&self, _: &str) -> Vec<u32> {
            Vec::new()
        }
        fn process_command_line(&self, _: u32) -> Option<String> {
            None
        }
        fn game_observe(&self, _: &str, _: Observation) {}
        fn game_claim(&self, _: &str, _: bool) {}
        fn game_event(
            &self,
            game: &str,
            kind: &str,
            _: &str,
            data: Map<String, Value>,
            _: Option<f64>,
        ) {
            self.events
                .lock()
                .unwrap()
                .push((game.into(), kind.into(), data));
        }
        fn push(&self, push: Push) -> Result<PushOutcome, String> {
            let mut pushes = self.pushes.lock().unwrap();
            let seen = pushes.iter().any(|p| p.key == push.key);
            pushes.push(push);
            Ok(if seen {
                PushOutcome::Updated
            } else {
                PushOutcome::Posted
            })
        }
    }

    /// 这次测试自己的识别记录目录（在 TMP 下，不碰真实的 ~/.kotone）。
    fn temp_history() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kotone-voice-input-pushes-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    struct Rig {
        orch: Arc<Orchestrator>,
        entry: Arc<Entry>,
        host: Arc<TestHost>,
        history: std::path::PathBuf,
    }

    impl Rig {
        fn new() -> Self {
            Self::with_timeout(Duration::from_secs(10))
        }

        /// `inject_timeout`：发送的看门狗（超时那一种测试调小）。
        fn with_timeout(inject_timeout: Duration) -> Self {
            let mut settings = Settings::default();
            settings.history.mode = HistoryMode::KeepAll;
            settings.active_profile_id = Some("lol".into());
            let entry = Arc::new(Entry::default());
            let mut orch = Orchestrator::new(
                Arc::new(RwLock::new(settings)),
                Arc::new(EngineRegistry::new()),
                Arc::new(Ports),
                Arc::new(kotone_core::inject::StubInjector),
                Arc::new(Ports),
                Arc::new(Ports),
            )
            .with_voice_input()
            .with_send_to_game(entry.clone());
            let history = temp_history();
            orch.history_dir = Some(history.clone());
            orch.focus_restore_delay = Duration::ZERO;
            orch.toast_dwell = Duration::from_millis(1);
            orch.inject_timeout = inject_timeout;
            let orch = orch.into_arc();
            let host = Arc::new(TestHost::default());
            orch.voice_input().unwrap().pushes.use_host(host.clone());
            Self {
                orch,
                entry,
                host,
                history,
            }
        }

        fn voice(&self) -> &crate::VoiceInput {
            self.orch.voice_input().unwrap()
        }

        /// 这一句当成听写的一句发（开始收音时在窗口 42、频道选的是所有人），识别记录有草稿。
        async fn say(&self, id: &str, text: &str) -> Result<ChatOutcome, String> {
            let voice = self.voice();
            let settings = self.orch.settings().read().unwrap().clone();
            voice.state().history = Some(Arc::new(DraftCell::new(HistoryDraft::new(
                id.into(),
                "test",
                &settings,
            ))));
            voice
                .state()
                .history
                .as_ref()
                .unwrap()
                .lock()
                .unwrap()
                .final_text = text.into();
            let claim = voice
                .claim_send(
                    &self.orch,
                    Some(ReadyText::unchanged(text.into())),
                    None,
                    &[
                        OrchestratorState::Idle,
                        OrchestratorState::Success,
                        OrchestratorState::Error,
                    ],
                    false,
                    None,
                    Route {
                        id: Some(id.into()),
                        place: Some((Some(TargetWindow(42)), Some(("lol".into(), "all".into())))),
                        ..Route::default()
                    },
                )
                .await
                .unwrap()
                .unwrap();
            let result = voice.run_send(&self.orch, claim).await;
            voice.pushes.settled().await;
            result
        }

        async fn resend(&self, id: &str, hint: Option<(&str, &str)>) -> Resent {
            let resent = self
                .voice()
                .resend_card(&self.orch, id, hint.map(|(g, c)| (g.into(), c.into())))
                .await;
            self.voice().pushes.settled().await;
            resent
        }

        fn records(&self) -> Vec<HistoryRecord> {
            kotone_core::history::list_in(&self.history).unwrap()
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.history);
        }
    }

    /// 发出去一句：一张“发出去的话”（并进同一张）、一件 sent 事件；识别记录写上发到了哪个游戏、哪个频道。
    #[tokio::test]
    async fn sent_lines_become_one_card_an_event_and_a_record_with_game_and_channel() {
        let rig = Rig::new();
        rig.say("s1", "打野来中路").await.unwrap();
        rig.say("s2", "小龙刷了").await.unwrap();
        let sent: Vec<Push> = rig
            .host
            .pushes()
            .into_iter()
            .filter(|p| p.trigger == PUSH_SENT)
            .collect();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].key, sent[1].key, "连着发、同一个频道：同一张卡片");
        let lines = sent[1].card.as_ref().unwrap().data["lines"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(lines.len(), 2);
        assert!(sent[1].note.contains("小龙刷了"));
        let events = rig.host.events.lock().unwrap().clone();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0, "lol");
        assert_eq!(events[0].1, "voice-input.sent");
        assert_eq!(events[0].2["channelName"], "所有人");

        let records = rig.records();
        assert_eq!(records[0].session_id, "s2");
        assert_eq!(records[0].outcome, HistoryOutcome::Sent);
        assert_eq!(records[0].game.as_ref().unwrap().name, "英雄联盟");
        assert_eq!(records[0].channel.as_ref().unwrap().id, "all");
        // 本体的入口拿到的：玩家自己的话、回到开始收音的窗口、不是重发（不 strict）
        let asked = rig.entry.asked();
        assert_eq!(asked[0].speaker, Speaker::Player);
        assert_eq!(asked[0].restore.map(|r| r.window), Some(TargetWindow(42)));
        assert!(!asked[0].strict_target);
    }

    /// 每一种没发出去（修饰键按着、键盘被占着、客户端没发、被拦截、要管理员、注入失败、超时、发送线程异常）都推一张
    /// “没发出去”，写着给人看的原因和代号，报一件 failed 事件；识别记录照旧是 error，没有去处。
    #[tokio::test]
    async fn every_kind_of_failure_becomes_a_failed_card_with_its_reason() {
        type Arrange = Box<dyn Fn(&Entry) + Send + Sync>;
        let cases: Vec<(&str, Arrange)> = vec![
            (
                "MODIFIERS_STILL_DOWN",
                Box::new(|e: &Entry| e.answer(|| Err(ChatFailure::ModifiersHeld))),
            ),
            (
                "INPUT_BUSY",
                Box::new(|e: &Entry| {
                    e.answer(|| {
                        Err(ChatFailure::Busy(InputBusy {
                            holder: Some(InputUser::GameAct),
                            waited_ms: 5000,
                        }))
                    })
                }),
            ),
            (
                "CLIENT_CHAT_FAILED",
                Box::new(|e: &Entry| {
                    e.answer(|| {
                        Err(ChatFailure::Provider(ChatStop::new(
                            StopCode::ClientRejected,
                            "客户端没收",
                        )))
                    })
                }),
            ),
            (
                "INJECTION_BLOCKED",
                Box::new(|e: &Entry| {
                    e.answer(|| {
                        Err(ChatFailure::Inject(InjectError::with_input_blocked(
                            "输入被系统拦截",
                        )))
                    })
                }),
            ),
            (
                "INJECTION_ELEVATION_REQUIRED",
                Box::new(|e: &Entry| {
                    e.answer(|| {
                        Err(ChatFailure::Inject(InjectError::with_needs_elevation(
                            "游戏以管理员身份运行",
                        )))
                    })
                }),
            ),
            (
                "INJECTION_FAILED",
                Box::new(|e: &Entry| {
                    e.answer(|| Err(ChatFailure::Inject(InjectError::new("打字失败"))))
                }),
            ),
            (
                "INJECTION_FAILED",
                Box::new(|e: &Entry| {
                    e.answer(|| {
                        std::thread::sleep(Duration::from_millis(400));
                        Err(ChatFailure::TimeUp)
                    })
                }),
            ),
            (
                "INJECTION_FAILED",
                Box::new(|e: &Entry| e.answer(|| panic!("发送线程挂了"))),
            ),
        ];
        for (i, (code, arrange)) in cases.into_iter().enumerate() {
            // 超时那一种：看门狗先到
            let rig = if i == 6 {
                Rig::with_timeout(Duration::from_millis(100))
            } else {
                Rig::new()
            };
            arrange(&rig.entry);
            let id = format!("f{i}");
            let reason = rig.say(&id, "打野来中路").await.unwrap_err();
            let push = rig.host.last(PUSH_FAILED);
            assert_eq!(push.key, format!("failed:{id}"));
            let card = push.card.as_ref().unwrap();
            assert_eq!(card.uri, PAGE_FAILED);
            assert_eq!(card.data["reason"], reason.as_str(), "{code}");
            assert_eq!(card.data["code"], code, "{reason}");
            assert_eq!(card.data["text"], "打野来中路");
            assert!(push.note.is_empty());
            if i == 6 {
                assert!(reason.contains("发送超时"), "{reason}");
            }
            if i == 7 {
                assert!(reason.contains("发送线程异常"), "{reason}");
            }
            let events = rig.host.events.lock().unwrap().clone();
            assert_eq!(events.last().unwrap().1, "voice-input.failed");
            assert_eq!(events.last().unwrap().2["reason"], reason.as_str());
            let record = &rig.records()[0];
            assert_eq!(record.outcome, HistoryOutcome::Error);
            assert!(record.game.is_none() && record.channel.is_none());
            assert_eq!(rig.orch.state(), OrchestratorState::Error);
        }
    }

    /// 卡片上的重发：经本体的入口，回到当时的窗口（42）、用当时的频道（所有人），只发到那个窗口（strict）；发出去以后
    /// 那张“没发出去”原地标成已重发，另外进“发出去的话”；识别记录照原来那条再记一条 sent（带去处）。再点一次不再发。
    #[tokio::test]
    async fn the_card_resends_through_the_entry_to_the_window_and_channel_it_was_meant_for() {
        let rig = Rig::new();
        rig.entry.answer(|| Err(ChatFailure::ModifiersHeld));
        rig.say("s1", "打野来中路").await.unwrap_err();
        assert_eq!(
            rig.orch.state(),
            OrchestratorState::Error,
            "输入条上停着这一句"
        );

        let resent = rig.resend("s1", None).await;
        let Resent::Sent(outcome) = resent else {
            panic!("{resent:?}");
        };
        assert_eq!(outcome.channel_name, "所有人");
        let asked = rig.entry.asked();
        assert_eq!(asked.len(), 2);
        let again = &asked[1];
        assert_eq!(again.text, "打野来中路");
        assert_eq!(again.speaker, Speaker::Player);
        assert!(again.strict_target, "只发到要发去的那个窗口");
        assert_eq!(again.restore.map(|r| r.window), Some(TargetWindow(42)));
        assert_eq!(
            again.channel,
            Some(ChannelPick {
                id: "all".into(),
                for_game: Some("lol".into())
            })
        );
        let failed = rig.host.last(PUSH_FAILED);
        assert_eq!(failed.key, "failed:s1", "原地更新同一张");
        assert_eq!(
            failed.card.as_ref().unwrap().data["resent"]["channel"]["name"],
            "所有人"
        );
        assert_eq!(
            rig.host.last(PUSH_SENT).card.as_ref().unwrap().data["lines"][0]["text"],
            "打野来中路"
        );

        let records = rig.records();
        assert_eq!(records.len(), 2, "失败一条、重发一条");
        assert_eq!(records[0].session_id, "s1");
        assert_eq!(records[0].outcome, HistoryOutcome::Sent);
        assert_eq!(records[0].channel.as_ref().unwrap().name, "所有人");
        assert_eq!(records[1].outcome, HistoryOutcome::Error);

        let twice = rig.resend("s1", None).await;
        assert!(
            matches!(twice, Resent::Refused(ref r) if r.contains("已经重发过了")),
            "{twice:?}"
        );
        assert_eq!(rig.entry.asked().len(), 2, "没再发");
    }

    /// 重发切不回要发去的窗口（入口的 strict_target 拒了）：一个键没按，卡片原地换成新的原因、次数加一，还能再点。
    #[tokio::test]
    async fn a_resend_that_cannot_reach_the_game_window_updates_the_card() {
        let rig = Rig::new();
        rig.entry.answer(|| Err(ChatFailure::ModifiersHeld));
        rig.say("s1", "打野来中路").await.unwrap_err();
        rig.entry.answer(|| Err(ChatFailure::WindowGone));
        let resent = rig.resend("s1", None).await;
        assert!(
            matches!(resent, Resent::Failed(ref r) if r == NO_GAME_WINDOW),
            "{resent:?}"
        );
        let failed = rig.host.last(PUSH_FAILED);
        assert_eq!(failed.key, "failed:s1");
        let data = &failed.card.as_ref().unwrap().data;
        assert_eq!(data["reason"], NO_GAME_WINDOW);
        assert_eq!(data["code"], "TARGET_WINDOW_GONE");
        assert_eq!(data["attempts"], 2);
        assert!(data.get("resent").is_none());
        // 还能再点：这次发出去了
        assert!(matches!(rig.resend("s1", None).await, Resent::Sent(_)));
    }

    /// Kotone 重启过（内存里没有这一句）：从识别记录里找回那句话，用卡片上记的频道；不知道当时的窗口，就不切窗口、
    /// 只发到现在的前台游戏（strict：前台是 Kotone 自己就不发）。找不到的、早就发出去了的不发。
    #[tokio::test]
    async fn after_a_restart_the_card_resends_from_the_history() {
        let rig = Rig::new();
        let cfg = rig.orch.settings().read().unwrap().history.clone();
        let record = |id: &str, outcome: HistoryOutcome| HistoryRecord {
            session_id: id.into(),
            ts: "2026-11-26T10:00:00Z".into(),
            engine_id: "test".into(),
            profile_id: Some("lol".into()),
            final_text: "打野来中路".into(),
            source_text: Some("打野来中路吧".into()),
            process_duration_ms: None,
            audio_ms: 0,
            first_partial_ms: None,
            finalize_latency_ms: None,
            outcome,
            error: None,
            audio_file: None,
            game: None,
            channel: None,
        };
        kotone_core::history::append_in(&rig.history, &record("old", HistoryOutcome::Error), &cfg)
            .unwrap();
        kotone_core::history::append_in(&rig.history, &record("done", HistoryOutcome::Sent), &cfg)
            .unwrap();

        assert!(matches!(
            rig.resend("old", Some(("lol", "all"))).await,
            Resent::Sent(_)
        ));
        let asked = rig.entry.asked();
        assert_eq!(asked[0].text, "打野来中路");
        assert!(asked[0].strict_target && asked[0].restore.is_none());
        assert_eq!(
            asked[0].channel.as_ref().map(|c| c.id.as_str()),
            Some("all")
        );
        assert_eq!(rig.records()[0].session_id, "old");
        assert_eq!(rig.records()[0].outcome, HistoryOutcome::Sent);

        assert!(matches!(rig.resend("done", None).await, Resent::Refused(_)));
        assert!(matches!(rig.resend("nope", None).await, Resent::Refused(_)));
        assert_eq!(rig.entry.asked().len(), 1);
    }

    /// 重发最近一条热键是新的一次发送：没发出去时是一张新的卡片，不更新上一句的。
    #[tokio::test]
    async fn the_resend_last_hotkey_gets_its_own_card() {
        let rig = Rig::new();
        rig.say("s1", "打野来中路").await.unwrap();
        // 等“已发送”的提示收起（重发最近一条热键只在空着时生效）
        let started = std::time::Instant::now();
        while rig.orch.state() != OrchestratorState::Idle {
            assert!(started.elapsed() < Duration::from_secs(2));
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        rig.entry.answer(|| Err(ChatFailure::ModifiersHeld));
        rig.voice().resend_last(&rig.orch).await;
        rig.voice().pushes.settled().await;
        let failed = rig.host.last(PUSH_FAILED);
        assert!(failed.key.starts_with("failed:resend-"), "{}", failed.key);
        assert!(rig.host.pushes().iter().all(|p| p.key != "failed:s1"));
        assert_eq!(HistoryWrite::default(), HistoryWrite::Draft);
    }

    /// 页面调的工具：参数不对直接说。
    #[tokio::test]
    async fn the_resend_tool_needs_an_id() {
        use kotone_plugin_api::Plugin;
        let plugin = crate::VoiceInputPlugin::new();
        let host: Arc<dyn Host> = Arc::new(TestHost::default());
        let error = plugin
            .call(host, crate::pushes::TOOL_RESEND, &json!({}))
            .await
            .unwrap_err();
        assert!(error.contains("id"), "{error}");
    }
}
