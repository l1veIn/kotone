//! 发到游戏的入口：假注入器 / 假前台 / 假时钟，不发系统输入、不真睡、不写盘。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use super::*;
use crate::inject::TargetWindow;

/// 睡觉只往前拨，记下每次睡多久。
struct FakeClock {
    now: Mutex<Instant>,
    slept: Mutex<Vec<u64>>,
}

impl FakeClock {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(Instant::now()),
            slept: Mutex::new(Vec::new()),
        })
    }
    fn advance(&self, ms: u64) {
        *self.now.lock().unwrap() += Duration::from_millis(ms);
    }
    fn slept(&self) -> Vec<u64> {
        self.slept.lock().unwrap().clone()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap()
    }
    fn sleep(&self, duration: Duration) {
        *self.now.lock().unwrap() += duration;
        self.slept.lock().unwrap().push(duration.as_millis() as u64);
    }
}

type Hook = Box<dyn FnMut(usize) + Send>;

/// 前台、修饰键、切回窗口；和注入器共用一份流水。
struct Screen {
    events: Arc<Mutex<Vec<String>>>,
    fg: Mutex<Option<ForegroundInfo>>,
    /// 每看一次修饰键弹出一个；空了 = 松开
    modifiers: Mutex<VecDeque<bool>>,
    always_down: AtomicBool,
    /// `window_info`：不在前台的窗口（语音输入开始收音的那个）
    windows: Mutex<Vec<ForegroundInfo>>,
}

impl FocusBackend for Screen {
    fn foreground_window(&self) -> Option<TargetWindow> {
        self.fg.lock().unwrap().as_ref().map(|f| f.window)
    }
    fn restore(&self, target: TargetWindow) -> bool {
        self.events
            .lock()
            .unwrap()
            .push(format!("restore:{}", target.0));
        true
    }
    fn foreground_process_name(&self) -> Option<String> {
        self.fg
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|f| f.process.clone())
    }
    fn modifier_keys_down(&self) -> bool {
        self.always_down.load(Ordering::SeqCst)
            || self.modifiers.lock().unwrap().pop_front().unwrap_or(false)
    }
    fn foreground_info(&self) -> Option<ForegroundInfo> {
        self.fg.lock().unwrap().clone()
    }
    fn window_info(&self, window: TargetWindow) -> Option<ForegroundInfo> {
        self.windows
            .lock()
            .unwrap()
            .iter()
            .find(|w| w.window == window)
            .cloned()
    }
}

/// 记下每一行（开框键|文字|发送键）的注入器；可以设走哪条路、第几行失败、每行之后做点什么。
struct Keys {
    events: Arc<Mutex<Vec<String>>>,
    route: SendRoute,
    fail_on: Option<usize>,
    calls: Mutex<usize>,
    after: Mutex<Option<Hook>>,
}

impl Injector for Keys {
    fn send(&self, _: &str, _: &GameProfile, _: CancelToken) -> Result<(), InjectError> {
        unreachable!("入口调 send_line")
    }
    fn send_line(
        &self,
        text: &str,
        profile: &GameProfile,
        _cancel: CancelToken,
    ) -> Result<SendRoute, InjectError> {
        let n = {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            *calls - 1
        };
        if self.fail_on == Some(n) {
            return Err(InjectError::with_input_blocked("输入被系统拦截"));
        }
        self.events.lock().unwrap().push(format!(
            "{}|{text}|{}",
            profile.open_chat_key, profile.send_key
        ));
        if let Some(hook) = self.after.lock().unwrap().as_mut() {
            hook(n);
        }
        Ok(self.route)
    }
}

struct Rig {
    chat: GameChat,
    clock: Arc<FakeClock>,
    screen: Arc<Screen>,
    keys: Arc<Keys>,
    events: Arc<Mutex<Vec<String>>>,
}

impl Rig {
    fn new(active: Option<&str>, fg: Option<ForegroundInfo>) -> Self {
        Self::with(active, fg, SendRoute::Keys, None)
    }

    fn with(
        active: Option<&str>,
        fg: Option<ForegroundInfo>,
        route: SendRoute,
        fail_on: Option<usize>,
    ) -> Self {
        Self::build(active, fg, route, fail_on, Vec::new())
    }

    fn build(
        active: Option<&str>,
        fg: Option<ForegroundInfo>,
        route: SendRoute,
        fail_on: Option<usize>,
        providers: Vec<Arc<dyn ChatProvider>>,
    ) -> Self {
        let events = Arc::new(Mutex::new(Vec::new()));
        let screen = Arc::new(Screen {
            events: events.clone(),
            fg: Mutex::new(fg),
            modifiers: Mutex::new(VecDeque::new()),
            always_down: AtomicBool::new(false),
            windows: Mutex::new(Vec::new()),
        });
        let keys = Arc::new(Keys {
            events: events.clone(),
            route,
            fail_on,
            calls: Mutex::new(0),
            after: Mutex::new(None),
        });
        let settings = Settings {
            active_profile_id: active.map(String::from),
            ..Settings::default()
        };
        let clock = FakeClock::new();
        let chat = GameChat::new(
            keys.clone(),
            screen.clone(),
            InputLock::new(),
            Arc::new(RwLock::new(settings)),
        )
        .with_profiles(games)
        .with_clock(clock.clone())
        .with_note(|_, _| {})
        .with_providers(providers);
        Self {
            chat,
            clock,
            screen,
            keys,
            events,
        }
    }

    fn after_line(&self, hook: impl FnMut(usize) + Send + 'static) {
        *self.keys.after.lock().unwrap() = Some(Box::new(hook));
    }

    fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
}

fn games() -> Vec<GameProfile> {
    vec![
        GameProfile::builtin_lol(),
        GameProfile::builtin_dota2(),
        GameProfile::builtin_cs2(),
        GameProfile::builtin_generic(),
    ]
}

fn fg(process: &str) -> Option<ForegroundInfo> {
    Some(ForegroundInfo {
        window: TargetWindow(7),
        process: Some(process.into()),
        own_process: false,
    })
}

const LOL: &str = "League of Legends.exe";

fn ask(text: &str, speaker: Speaker) -> ChatRequest {
    ChatRequest::new(text, speaker)
}

fn all(for_game: Option<&str>) -> Option<ChannelPick> {
    Some(ChannelPick {
        id: CHANNEL_ALL.into(),
        for_game: for_game.map(String::from),
    })
}

// ---------- 挑游戏配置（前台检测） ----------

#[test]
fn one_line_goes_to_the_default_channel_of_the_foreground_game() {
    let rig = Rig::new(Some("lol"), fg(LOL));
    let out = rig.chat.send(ask("  gg  ", Speaker::Companion)).unwrap();
    assert_eq!(rig.events(), ["Enter|gg|Enter"]);
    assert_eq!((out.game.as_str(), out.channel.as_str()), ("lol", "team"));
    assert_eq!(out.channel_name, "队伍");
    assert!(out.sent && out.sent_lines == 1 && out.stopped.is_none());
    assert_eq!(out.route, SendRoute::Keys);
    let json = out.to_json();
    assert_eq!(json["route"], "keys");
    assert_eq!(json["channel"], "team");
    assert_eq!(json["sentLines"], 1);
    assert!(json.get("stopped").is_none() && json.get("channelFellBack").is_none());
}

#[test]
fn the_foreground_game_wins_over_the_selected_one() {
    // 选中英雄联盟、前台是 Dota 2：按 Dota 2 的配置发（今天按英雄联盟的发）
    let rig = Rig::new(Some("lol"), fg("dota2.exe"));
    let out = rig.chat.send(ask("推中", Speaker::Companion)).unwrap();
    assert_eq!(out.game, "dota2");
    // CS2 开聊天框是 U：配置真的换了
    let rig = Rig::new(Some("lol"), fg("cs2.exe"));
    rig.chat.send(ask("rush B", Speaker::Agent)).unwrap();
    assert_eq!(rig.events(), ["U|rush B|Enter"]);
}

#[test]
fn speaking_for_the_player_needs_a_game_in_front_but_the_player_is_never_refused() {
    let rig = Rig::new(Some("lol"), fg("chrome.exe"));
    for speaker in [Speaker::Companion, Speaker::Agent, Speaker::Automation] {
        let err = rig.chat.send(ask("gg", speaker)).unwrap_err();
        assert!(matches!(err, ChatFailure::NotAGame { .. }), "{err:?}");
        assert_eq!(
            err.to_string(),
            "前台是 chrome.exe，不是「League of Legends」：请先切到游戏，再让我替你发言"
        );
    }
    assert!(rig.events().is_empty(), "一个键都没按");
    // 语音输入：玩家自己在这个窗口按的键，照旧发（用选中的配置）
    let out = rig.chat.send(ask("gg", Speaker::Player)).unwrap();
    assert_eq!(out.game, "lol");
    assert_eq!(rig.events(), ["Enter|gg|Enter"]);
}

#[test]
fn foreground_refusals_are_the_old_rules() {
    // 原来桌面端 foreground_refusal / foreground_target_info 的规则（companion.rs 的测试搬过来）
    let lol = GameProfile::builtin_lol();
    let generic = GameProfile::builtin_generic();
    let list = games();
    let pick = |f: Option<ForegroundInfo>, active: &GameProfile| {
        pick_game(f.as_ref(), &list, active, Speaker::Companion)
    };
    assert!(matches!(
        pick(fg("explorer.exe"), &lol),
        Err(ChatFailure::Desktop)
    ));
    assert!(matches!(pick(None, &lol), Err(ChatFailure::NoForeground)));
    let nameless = Some(ForegroundInfo {
        window: TargetWindow(1),
        process: None,
        own_process: false,
    });
    assert!(matches!(pick(nameless, &lol), Err(ChatFailure::Desktop)));
    assert!(matches!(
        pick(fg("notepad.exe"), &lol),
        Err(ChatFailure::NotAGame { .. })
    ));
    assert_eq!(pick(fg(LOL), &lol).unwrap().id, "lol");
    let own = Some(ForegroundInfo {
        window: TargetWindow(1),
        process: Some("kotone.exe".into()),
        own_process: true,
    });
    assert!(matches!(pick(own, &generic), Err(ChatFailure::OwnWindow)));
    // 选中“通用”：除了桌面和 Kotone 自己，前台是什么都行
    assert_eq!(pick(fg("SomeGame.exe"), &generic).unwrap().id, "generic");
    assert!(matches!(
        pick(fg("Explorer.EXE"), &generic),
        Err(ChatFailure::Desktop)
    ));
    // 玩家：没有前台信息也照旧用选中的
    assert_eq!(
        pick_game(None, &list, &lol, Speaker::Player).unwrap().id,
        "lol"
    );
}

#[test]
fn nothing_is_read_from_disk_when_nothing_needs_it() {
    // 没选中游戏、不知道前台是谁：直接用通用配置，不列游戏配置（编排器测试不碰 ~/.kotone）
    let read = Arc::new(AtomicBool::new(false));
    let flag = read.clone();
    let rig = Rig::new(None, None);
    let chat = rig.chat.with_profiles(move || {
        flag.store(true, Ordering::SeqCst);
        games()
    });
    let out = chat.send(ask("hi", Speaker::Player)).unwrap();
    assert_eq!(out.game, "generic");
    assert!(!read.load(Ordering::SeqCst));
}

// ---------- 修饰键（评估 #2） ----------

#[test]
fn held_modifiers_are_waited_for_then_the_line_is_sent() {
    let rig = Rig::new(Some("lol"), fg(LOL));
    rig.screen
        .modifiers
        .lock()
        .unwrap()
        .extend([true, true, true]);
    let out = rig.chat.send(ask("gg", Speaker::Companion)).unwrap();
    assert!(out.sent);
    assert_eq!(rig.events(), ["Enter|gg|Enter"]);
    assert!(out.waited_ms >= 20, "{out:?}");
}

#[test]
fn modifiers_held_for_a_second_send_nothing() {
    let rig = Rig::new(Some("lol"), fg(LOL));
    rig.screen.always_down.store(true, Ordering::SeqCst);
    let started = rig.clock.now();
    let err = rig.chat.send(ask("gg", Speaker::Player)).unwrap_err();
    assert!(matches!(err, ChatFailure::ModifiersHeld), "{err:?}");
    assert_eq!(err.code(), "modifiersHeld");
    assert!(rig.events().is_empty(), "一个键都没按，也没切窗口");
    assert!(rig.clock.now() - started >= MODIFIER_WAIT);
}

#[test]
fn shift_pressed_between_lines_is_waited_for_too() {
    let rig = Rig::new(Some("lol"), fg(LOL));
    let screen = rig.screen.clone();
    rig.after_line(move |n| {
        if n == 0 {
            // 第一行发完、第二行之前玩家按下 Shift（这时挡键窗口是关的）
            screen.modifiers.lock().unwrap().extend([true, true]);
        }
    });
    let out = rig
        .chat
        .send(ask("第一行\n第二行", Speaker::Companion))
        .unwrap();
    assert_eq!(rig.events(), ["Enter|第一行|Enter", "Enter|第二行|Enter"]);
    assert!(out.sent);
    let slept = rig.clock.slept();
    assert_eq!(slept[0], 65, "行间隔：{slept:?}");
    assert!(slept[1..].contains(&10), "第二行之前等了修饰键：{slept:?}");
}

// ---------- 频道 ----------

#[test]
fn all_uses_the_all_channel_key_and_a_game_without_all_sends_nothing() {
    let rig = Rig::new(Some("lol"), fg(LOL));
    let mut request = ask("gg", Speaker::Companion);
    request.channel = all(None);
    let out = rig.chat.send(request).unwrap();
    assert_eq!(rig.events(), ["Shift+Enter|gg|Enter"]);
    assert_eq!(
        (out.channel.as_str(), out.channel_name.as_str()),
        ("all", "所有人")
    );

    // 通用配置没有“所有人”：发给更多人不能悄悄退
    let rig = Rig::new(Some("generic"), fg("SomeGame.exe"));
    let mut request = ask("gg", Speaker::Companion);
    request.channel = all(None);
    let err = rig.chat.send(request).unwrap_err();
    assert!(matches!(err, ChatFailure::NoSuchChannel { .. }), "{err:?}");
    assert!(err.to_string().contains("没有「所有人」频道"), "{err}");
    assert!(rig.events().is_empty());
    // 要“队伍”而没有：发给更少的人，退回默认频道
    let mut request = ask("gg", Speaker::Companion);
    request.channel = Some(ChannelPick {
        id: CHANNEL_TEAM.into(),
        for_game: None,
    });
    let out = rig.chat.send(request).unwrap();
    assert_eq!(out.channel, "default");
    assert_eq!(rig.events(), ["Enter|gg|Enter"]);
}

#[test]
fn team_is_not_replaced_by_a_default_that_is_all() {
    let mut game = GameProfile::builtin_lol();
    game.channels.retain(|c| c.id == CHANNEL_ALL);
    game.channels[0].default = true;
    let pick = ChannelPick {
        id: CHANNEL_TEAM.into(),
        for_game: None,
    };
    assert!(matches!(
        pick_channel(&game, Some(&pick)),
        Err(ChatFailure::NoSuchChannel { .. })
    ));
}

#[test]
fn a_voice_channel_chosen_in_another_game_falls_back_to_the_default() {
    // 原来语音输入 resolve_send_strategy 的规则（插件的测试搬过来）
    let lol = GameProfile::builtin_lol();
    let (s, fell_back) = pick_channel(&lol, all(Some("lol")).as_ref()).unwrap();
    assert_eq!(
        (s.id.as_str(), s.open_chat_key.as_str()),
        ("all", "Shift+Enter")
    );
    assert!(!fell_back);
    let generic = GameProfile::builtin_generic();
    let (s, fell_back) = pick_channel(&generic, all(Some("lol")).as_ref()).unwrap();
    assert!(s.is_default && fell_back);
    assert_eq!(s.open_chat_key, generic.open_chat_key);
    let (s, fell_back) = pick_channel(&lol, None).unwrap();
    assert_eq!(s.id, "team");
    assert!(!fell_back);
    // 选的是默认频道本身：谈不上“退回”
    let team_in_lol = ChannelPick {
        id: CHANNEL_TEAM.into(),
        for_game: Some("lol".into()),
    };
    let (s, fell_back) = pick_channel(&GameProfile::builtin_dota2(), Some(&team_in_lol)).unwrap();
    assert_eq!(s.id, "team");
    assert!(!fell_back);
}

#[test]
fn the_chosen_channel_picks_the_console_command() {
    let mut dota = GameProfile::builtin_dota2();
    dota.channels[0].console_command = Some("say_team".into());
    dota.channels[1].console_command = Some("say".into());
    let (s, _) = pick_channel(&dota, all(None).as_ref()).unwrap();
    let wire = dota.for_channel(&s);
    assert_eq!(wire.open_chat_key, "Shift+Enter");
    assert_eq!(wire.channel_strategy(None).console_command, "say");
    let (s, _) = pick_channel(&dota, None).unwrap();
    assert_eq!(
        dota.for_channel(&s).channel_strategy(None).console_command,
        "say_team"
    );
}

// ---------- 多行 ----------

#[test]
fn lines_are_split_trimmed_and_limited() {
    assert_eq!(
        split_lines(" 亚索 2/0/1 \r\n\n 劫 0/2/0\r", true).unwrap(),
        ["亚索 2/0/1", "劫 0/2/0"]
    );
    assert!(matches!(
        split_lines(" \n \r\n", true),
        Err(ChatFailure::EmptyText)
    ));
    let eleven = ["x"; 11].join("\n");
    assert!(matches!(
        split_lines(&eleven, true),
        Err(ChatFailure::TooManyLines { lines: 11, max: 10 })
    ));
    assert!(split_lines(&["x"; 10].join("\n"), true).is_ok());
    let long = format!("ok\n{}", "字".repeat(201));
    assert!(matches!(
        split_lines(&long, true),
        Err(ChatFailure::LineTooLong { line: 2, max: 200 })
    ));
    assert!(split_lines(&"字".repeat(200), true).is_ok());
    assert!(matches!(
        split_lines("a\nb", false),
        Err(ChatFailure::MultiLineWithoutSend)
    ));
    assert!(split_lines("a", false).is_ok());
}

#[test]
fn lines_go_out_in_order_with_65_ms_between_them() {
    let rig = Rig::new(Some("lol"), fg(LOL));
    let out = rig
        .chat
        .send(ask("一\n二\n三", Speaker::Companion))
        .unwrap();
    assert_eq!(
        rig.events(),
        ["Enter|一|Enter", "Enter|二|Enter", "Enter|三|Enter"]
    );
    assert_eq!(rig.clock.slept(), [65, 65], "最后一行之后不等");
    assert_eq!((out.lines.len(), out.sent_lines), (3, 3));
}

#[test]
fn bad_text_presses_nothing() {
    let rig = Rig::new(Some("lol"), fg(LOL));
    let mut request = ask("a\nb", Speaker::Companion);
    request.send = false;
    let err = rig.chat.send(request).unwrap_err();
    assert!(err.is_invalid_arguments());
    assert!(rig.events().is_empty());
}

#[test]
fn send_false_types_without_the_send_key() {
    let rig = Rig::new(Some("lol"), fg(LOL));
    let mut request = ask("先别发", Speaker::Agent);
    request.send = false;
    let out = rig.chat.send(request).unwrap();
    assert_eq!(rig.events(), ["Enter|先别发|"]);
    assert!(out.sent);
}

// ---------- 停下来的几种情况：说清楚，不重试 ----------

#[test]
fn a_foreground_change_between_lines_stops_and_says_what_was_not_sent() {
    let rig = Rig::new(Some("lol"), fg(LOL));
    let screen = rig.screen.clone();
    rig.after_line(move |_| {
        // 第一行发完，玩家切到了别的窗口
        *screen.fg.lock().unwrap() = Some(ForegroundInfo {
            window: TargetWindow(99),
            process: Some("chrome.exe".into()),
            own_process: false,
        });
    });
    let out = rig.chat.send(ask("一\n二", Speaker::Companion)).unwrap();
    assert_eq!(rig.events(), ["Enter|一|Enter"], "没往新窗口里按键");
    assert!(!out.sent);
    assert_eq!(out.sent_lines, 1);
    assert!(matches!(out.stopped, Some(ChatFailure::ForegroundChanged)));
    let json = out.to_json();
    assert_eq!(json["sent"], false);
    assert_eq!(json["stopped"]["code"], "foregroundChanged");
    assert_eq!(
        json["stopped"]["message"],
        "后面 1 行没发：前台换了窗口，没往新窗口里发"
    );
}

#[test]
fn an_injection_failure_on_the_first_line_is_an_error_and_later_is_partial() {
    let rig = Rig::with(Some("lol"), fg(LOL), SendRoute::Keys, Some(0));
    let err = rig
        .chat
        .send(ask("一\n二", Speaker::Companion))
        .unwrap_err();
    assert_eq!(err.code(), "inputBlocked");
    assert!(rig.events().is_empty());

    let rig = Rig::with(Some("lol"), fg(LOL), SendRoute::Keys, Some(1));
    let out = rig
        .chat
        .send(ask("一\n二\n三", Speaker::Companion))
        .unwrap();
    assert_eq!(out.sent_lines, 1);
    assert_eq!(rig.events(), ["Enter|一|Enter"], "失败以后不再发、不重试");
    assert!(matches!(out.stopped, Some(ChatFailure::Inject(_))));
}

#[test]
fn no_line_is_started_that_cannot_finish_before_the_deadline() {
    // 期限太短：一行都不开始，一个键都不按
    let rig = Rig::new(Some("lol"), fg(LOL));
    let mut request = ask("gg", Speaker::Agent);
    request.deadline = Some(rig.clock.now() + Duration::from_millis(50));
    let err = rig.chat.send(request).unwrap_err();
    assert!(matches!(err, ChatFailure::TimeUp), "{err:?}");
    assert!(rig.events().is_empty());

    // 够发一行、不够发两行：发一行，说时间到了（调用方拿到的结果是真的）
    let rig = Rig::new(Some("lol"), fg(LOL));
    let clock = rig.clock.clone();
    rig.after_line(move |_| clock.advance(300));
    let estimate = estimate_line(&GameProfile::builtin_lol(), "一");
    assert!(estimate < Duration::from_millis(600), "{estimate:?}");
    let mut request = ask("一\n二", Speaker::Agent);
    request.deadline = Some(rig.clock.now() + Duration::from_millis(600));
    let out = rig.chat.send(request).unwrap();
    assert_eq!(out.sent_lines, 1);
    assert!(matches!(out.stopped, Some(ChatFailure::TimeUp)));
    assert_eq!(out.to_json()["stopped"]["code"], "timeUp");
}

#[test]
fn cancelling_between_lines_stops() {
    let rig = Rig::new(Some("lol"), fg(LOL));
    let request = ask("一\n二", Speaker::Player);
    let cancel = request.cancel.clone();
    rig.after_line(move |_| cancel.cancel());
    let out = rig.chat.send(request).unwrap();
    assert_eq!(out.sent_lines, 1);
    assert!(matches!(out.stopped, Some(ChatFailure::Cancelled)));
}

#[test]
fn the_route_is_reported() {
    let rig = Rig::with(Some("dota2"), fg("dota2.exe"), SendRoute::Console, None);
    let out = rig.chat.send(ask("推中", Speaker::Companion)).unwrap();
    assert_eq!(out.route, SendRoute::Console);
    assert_eq!(out.to_json()["route"], "console");
}

// ---------- 切回窗口（D5）和锁 ----------

#[test]
fn going_back_to_the_window_happens_inside_the_entry_before_typing() {
    let rig = Rig::new(Some("lol"), fg(LOL));
    let mut request = ask("gg", Speaker::Player);
    request.restore = Some(RestoreFocus {
        window: TargetWindow(42),
        settle: Duration::from_millis(30),
    });
    rig.chat.send(request).unwrap();
    assert_eq!(rig.events(), ["restore:42", "Enter|gg|Enter"]);
    assert_eq!(rig.clock.slept(), [30]);
}

#[test]
fn modifiers_that_never_come_up_do_not_switch_the_window() {
    let rig = Rig::new(Some("lol"), fg(LOL));
    rig.screen.always_down.store(true, Ordering::SeqCst);
    let mut request = ask("gg", Speaker::Player);
    request.restore = Some(RestoreFocus {
        window: TargetWindow(42),
        settle: Duration::ZERO,
    });
    assert!(rig.chat.send(request).is_err());
    assert!(rig.events().is_empty(), "等修饰键失败：不切窗口");
}

#[test]
fn someone_else_holding_the_lock_past_the_deadline_is_busy_and_nothing_is_pressed() {
    let rig = Rig::new(Some("lol"), fg(LOL));
    let lock = rig.chat.lock().clone();
    let (held, release) = (
        Arc::new(std::sync::Barrier::new(2)),
        Arc::new(AtomicBool::new(false)),
    );
    let holder = {
        let (held, release) = (held.clone(), release.clone());
        std::thread::spawn(move || {
            let _guard = lock.acquire_for(InputUser::GameAct).unwrap();
            held.wait();
            while !release.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
        })
    };
    held.wait();
    let mut request = ask("gg", Speaker::Player);
    request.deadline = Some(rig.clock.now() + Duration::from_millis(100));
    let err = rig.chat.send(request).unwrap_err();
    release.store(true, Ordering::SeqCst);
    holder.join().unwrap();
    let ChatFailure::Busy(busy) = &err else {
        panic!("{err:?}")
    };
    assert_eq!(busy.holder, Some(InputUser::GameAct));
    assert!(err.to_string().starts_with("她正在操作游戏"), "{err}");
    assert!(rig.events().is_empty());
}

/// 评估 #3 的回归：两个人同时发，一个发完另一个才开始（真时间、真线程）。
#[test]
fn two_speakers_at_the_same_time_never_interleave() {
    struct Slow(Arc<Mutex<Vec<String>>>);
    impl Injector for Slow {
        fn send(&self, text: &str, _: &GameProfile, _: CancelToken) -> Result<(), InjectError> {
            for step in ["open", "text", "send"] {
                self.0.lock().unwrap().push(format!("{step}:{text}"));
                std::thread::sleep(Duration::from_millis(15));
            }
            Ok(())
        }
    }
    let log = Arc::new(Mutex::new(Vec::new()));
    let screen = Arc::new(Screen {
        events: Arc::new(Mutex::new(Vec::new())),
        fg: Mutex::new(fg(LOL)),
        modifiers: Mutex::new(VecDeque::new()),
        always_down: AtomicBool::new(false),
        windows: Mutex::new(Vec::new()),
    });
    let settings = Settings {
        active_profile_id: Some("lol".into()),
        ..Settings::default()
    };
    let chat = Arc::new(
        GameChat::new(
            Arc::new(Slow(log.clone())),
            screen,
            InputLock::new(),
            Arc::new(RwLock::new(settings)),
        )
        .with_profiles(games)
        .with_note(|_, _| {}),
    );
    let start = Arc::new(std::sync::Barrier::new(2));
    let speakers = [("A1\nA2", Speaker::Companion), ("B1", Speaker::Player)].map(|(text, who)| {
        let (chat, start) = (chat.clone(), start.clone());
        std::thread::spawn(move || {
            start.wait();
            chat.send(ChatRequest::new(text, who)).unwrap()
        })
    });
    for s in speakers {
        assert!(s.join().unwrap().sent);
    }
    let log = log.lock().unwrap().clone();
    let one = |t: &str| ["open", "text", "send"].map(|s| format!("{s}:{t}"));
    let a: Vec<String> = one("A1").into_iter().chain(one("A2")).collect();
    let b: Vec<String> = one("B1").into_iter().collect();
    let a_then_b: Vec<String> = a.iter().chain(&b).cloned().collect();
    let b_then_a: Vec<String> = b.iter().chain(&a).cloned().collect();
    assert!(log == a_then_b || log == b_then_a, "交错了：{log:?}");
}

// ---------- 插件的发法（D3，第 2 期） ----------

const CLIENT: &str = "LeagueClientUx.exe";

/// 假的“英雄联盟客户端”发法：登记客户端窗口，记下收到的每一次，按设好的结果回（不设 = 全发了）。
struct FakeClient {
    spec: SendToGameSpec,
    off: Mutex<Option<String>>,
    reply: Mutex<Option<Result<ChatSent, ChatStop>>>,
    got: Mutex<Vec<ChatSend>>,
}

impl ChatProvider for FakeClient {
    fn plugin(&self) -> &str {
        "lol"
    }
    fn spec(&self) -> &SendToGameSpec {
        &self.spec
    }
    fn off(&self) -> Option<String> {
        self.off.lock().unwrap().clone()
    }
    fn send(&self, request: ChatSend) -> Result<ChatSent, ChatStop> {
        let (lines, channel) = (request.lines.len(), request.channel.clone());
        self.got.lock().unwrap().push(request);
        self.reply.lock().unwrap().clone().unwrap_or(Ok(ChatSent {
            channel,
            channel_name: "选英雄的聊天".into(),
            sent_lines: lines,
            stopped: None,
        }))
    }
}

fn client_rig(fg: Option<ForegroundInfo>) -> (Rig, Arc<FakeClient>) {
    let client = Arc::new(FakeClient {
        spec: SendToGameSpec {
            game: "lol".into(),
            windows: vec![CLIENT.into()],
            needs: vec!["client".into(), "chat".into()],
        },
        off: Mutex::new(None),
        reply: Mutex::new(None),
        got: Mutex::new(Vec::new()),
    });
    let rig = Rig::build(
        Some("lol"),
        fg,
        SendRoute::Keys,
        None,
        vec![client.clone() as Arc<dyn ChatProvider>],
    );
    (rig, client)
}

fn asked(client: &FakeClient) -> usize {
    client.got.lock().unwrap().len()
}

#[test]
fn the_client_window_goes_to_the_plugin_and_no_key_is_pressed() {
    let (rig, client) = client_rig(fg(CLIENT));
    let out = rig
        .chat
        .send(ask("亚索 5/0/2\n劫 0/4/1", Speaker::Companion))
        .unwrap();
    assert!(rig.events().is_empty(), "一个键都没按：{:?}", rig.events());
    assert_eq!(out.route, SendRoute::Client);
    assert_eq!(out.plugin.as_deref(), Some("lol"));
    assert_eq!(
        (out.game.as_str(), out.game_name.as_str()),
        ("lol", "League of Legends")
    );
    assert_eq!(
        (out.channel.as_str(), out.channel_name.as_str()),
        ("team", "选英雄的聊天")
    );
    assert!(out.sent && out.sent_lines == 2 && out.stopped.is_none());
    let got = client.got.lock().unwrap();
    assert_eq!(got.len(), 1, "交给插件一次");
    assert_eq!(got[0].lines, ["亚索 5/0/2", "劫 0/4/1"], "宿主整理好的行");
    assert_eq!(
        (got[0].game.as_str(), got[0].channel.as_str()),
        ("lol", "team")
    );
    let json = out.to_json();
    assert_eq!(json["route"], "client");
    assert_eq!(json["plugin"], "lol");
    assert_eq!(json["sentLines"], 2);
    // 走通用发法时没有 plugin 字段（只加不改）
    let (rig, _) = client_rig(fg(LOL));
    assert!(rig
        .chat
        .send(ask("gg", Speaker::Companion))
        .unwrap()
        .to_json()
        .get("plugin")
        .is_none());
}

#[test]
fn the_plugin_takes_no_lock_waits_for_no_modifier_and_switches_no_window() {
    let (rig, client) = client_rig(fg(LOL));
    // 开始收音的窗口是客户端（现在前台已经是游戏）
    rig.screen.windows.lock().unwrap().push(ForegroundInfo {
        window: TargetWindow(42),
        process: Some(CLIENT.into()),
        own_process: false,
    });
    rig.screen.always_down.store(true, Ordering::SeqCst);
    let lock = rig.chat.lock().clone();
    let (held, release) = (
        Arc::new(std::sync::Barrier::new(2)),
        Arc::new(AtomicBool::new(false)),
    );
    let holder = {
        let (held, release) = (held.clone(), release.clone());
        std::thread::spawn(move || {
            let _guard = lock.acquire_for(InputUser::GameAct).unwrap();
            held.wait();
            while !release.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
        })
    };
    held.wait();
    let mut request = ask("我们打龙", Speaker::Player);
    request.restore = Some(RestoreFocus {
        window: TargetWindow(42),
        settle: Duration::from_millis(30),
    });
    let deadline = rig.clock.now() + Duration::from_millis(100);
    request.deadline = Some(deadline);
    let out = rig.chat.send(request);
    release.store(true, Ordering::SeqCst);
    holder.join().unwrap();
    let out = out.unwrap();
    assert_eq!(out.route, SendRoute::Client, "锁被占着、修饰键按着也照发");
    assert!(
        rig.events().is_empty(),
        "没切窗口、没按键：{:?}",
        rig.events()
    );
    assert!(rig.clock.slept().is_empty(), "没等修饰键、没等切窗口");
    assert_eq!(out.waited_ms, 0);
    assert_eq!(client.got.lock().unwrap()[0].deadline, deadline);
}

#[test]
fn the_window_where_the_player_started_talking_decides() {
    // 开始收音在游戏里、发的时候前台是客户端：照旧按键发回游戏（插件不管）
    let (rig, client) = client_rig(fg(CLIENT));
    rig.screen.windows.lock().unwrap().push(ForegroundInfo {
        window: TargetWindow(42),
        process: Some(LOL.into()),
        own_process: false,
    });
    let mut request = ask("gg", Speaker::Player);
    request.restore = Some(RestoreFocus {
        window: TargetWindow(42),
        settle: Duration::ZERO,
    });
    let out = rig.chat.send(request).unwrap();
    assert_eq!(out.route, SendRoute::Keys);
    assert_eq!(rig.events(), ["restore:42", "Enter|gg|Enter"]);
    assert_eq!(asked(&client), 0);
    // 开始收音的窗口没了：看现在的前台
    let (rig, client) = client_rig(fg(CLIENT));
    let mut request = ask("gg", Speaker::Player);
    request.restore = Some(RestoreFocus {
        window: TargetWindow(77),
        settle: Duration::ZERO,
    });
    assert_eq!(rig.chat.send(request).unwrap().route, SendRoute::Client);
    assert_eq!(asked(&client), 1);
}

#[test]
fn the_game_window_and_kotone_itself_are_not_the_plugins() {
    let (rig, client) = client_rig(fg(LOL));
    let out = rig.chat.send(ask("一\n二", Speaker::Companion)).unwrap();
    assert_eq!(out.route, SendRoute::Keys, "对局里照旧是通用发法");
    assert_eq!(rig.events(), ["Enter|一|Enter", "Enter|二|Enter"]);
    assert_eq!(asked(&client), 0);
    let own = Some(ForegroundInfo {
        window: TargetWindow(1),
        process: Some(CLIENT.into()),
        own_process: true,
    });
    let (rig, client) = client_rig(own);
    let err = rig.chat.send(ask("gg", Speaker::Companion)).unwrap_err();
    assert!(matches!(err, ChatFailure::OwnWindow), "{err:?}");
    assert_eq!(asked(&client), 0);
}

#[test]
fn a_switched_off_plugin_refuses_speaking_for_the_player_but_the_player_types() {
    let (rig, client) = client_rig(fg(CLIENT));
    *client.off.lock().unwrap() = Some("英雄联盟插件的「在客户端里发聊天」关着".into());
    for speaker in [Speaker::Companion, Speaker::Agent, Speaker::Automation] {
        let err = rig.chat.send(ask("gg", speaker)).unwrap_err();
        assert!(matches!(err, ChatFailure::ProviderOff { .. }), "{err:?}");
        assert_eq!(err.code(), "providerOff");
        assert_eq!(
            err.to_string(),
            "前台是 LeagueClientUx.exe，英雄联盟插件的「在客户端里发聊天」关着，这次没替你发：打开它，或者切到游戏里再让我替你发言"
        );
    }
    assert!(rig.events().is_empty(), "替玩家发言的不往客户端里按键");
    assert_eq!(asked(&client), 0);
    // 玩家自己的语音输入：前台是什么窗就往哪发（照旧按键）
    let out = rig.chat.send(ask("gg", Speaker::Player)).unwrap();
    assert_eq!(out.route, SendRoute::Keys);
    assert_eq!(rig.events(), ["Enter|gg|Enter"]);
    assert_eq!(asked(&client), 0);
}

#[test]
fn a_plugin_failure_is_final_and_never_falls_back_to_keys() {
    let (rig, client) = client_rig(fg(CLIENT));
    *client.reply.lock().unwrap() = Some(Err(ChatStop::new(
        StopCode::ClientNoReply,
        "客户端没在期限内回话：这句可能已经发出去了，先看一眼聊天，别再发一遍",
    )));
    let err = rig.chat.send(ask("gg", Speaker::Agent)).unwrap_err();
    assert!(matches!(err, ChatFailure::Provider(_)), "{err:?}");
    assert_eq!(err.code(), "clientNoReply");
    assert!(err.to_string().contains("可能已经发出去了"), "{err}");
    assert!(rig.events().is_empty(), "不退回按键");
    assert_eq!(asked(&client), 1, "只问一次，不重试");

    // 发了一部分：照实说
    *client.reply.lock().unwrap() = Some(Ok(ChatSent {
        channel: "team".into(),
        channel_name: "选英雄的聊天".into(),
        sent_lines: 1,
        stopped: Some(ChatStop::new(
            StopCode::ClientRejected,
            "客户端没收下第二行",
        )),
    }));
    let out = rig.chat.send(ask("一\n二", Speaker::Agent)).unwrap();
    assert!(!out.sent);
    assert_eq!(out.sent_lines, 1);
    let json = out.to_json();
    assert_eq!(json["sent"], false);
    assert_eq!(json["stopped"]["code"], "clientRejected");
    assert_eq!(
        json["stopped"]["message"],
        "后面 1 行没发：客户端没收下第二行"
    );

    // Ok 却一行都没发：当作没发
    *client.reply.lock().unwrap() = Some(Ok(ChatSent {
        channel: "team".into(),
        channel_name: "选英雄的聊天".into(),
        sent_lines: 0,
        stopped: None,
    }));
    let err = rig.chat.send(ask("gg", Speaker::Agent)).unwrap_err();
    assert_eq!(err.code(), "clientRejected");
    assert!(rig.events().is_empty());
}

#[test]
fn only_typing_into_the_chat_box_is_refused_where_the_plugin_sends() {
    let (rig, client) = client_rig(fg(CLIENT));
    let mut request = ask("先别发", Speaker::Companion);
    request.send = false;
    let err = rig.chat.send(request).unwrap_err();
    assert!(matches!(err, ChatFailure::NoDraftHere), "{err:?}");
    assert_eq!(err.code(), "noDraftHere");
    assert!(!err.is_invalid_arguments(), "不是参数写错了，是这里做不到");
    assert!(rig.events().is_empty());
    assert_eq!(asked(&client), 0);
}

#[test]
fn the_channel_handed_to_the_plugin_is_already_worked_out() {
    let (rig, client) = client_rig(fg(CLIENT));
    let send = |channel: Option<ChannelPick>| {
        let mut request = ask("gg", Speaker::Player);
        request.channel = channel;
        let out = rig.chat.send(request).unwrap();
        let got = client.got.lock().unwrap().last().unwrap().channel.clone();
        (got, out.channel_fell_back)
    };
    assert_eq!(
        send(None),
        ("team".to_string(), false),
        "缺省 = 英雄联盟的默认频道"
    );
    assert_eq!(send(all(None)), ("all".to_string(), false));
    assert_eq!(send(all(Some("lol"))), ("all".to_string(), false));
    // 语音输入记的是 Dota 2 的“所有人”：在英雄联盟里退回默认频道
    assert_eq!(send(all(Some("dota2"))), ("team".to_string(), true));
    let lol = GameProfile::builtin_lol();
    assert_eq!(requested_channel(&lol, None), ("team".to_string(), false));
}

#[test]
fn cancelled_before_the_plugin_is_asked_sends_nothing() {
    let (rig, client) = client_rig(fg(CLIENT));
    let request = ask("gg", Speaker::Player);
    request.cancel.cancel();
    let err = rig.chat.send(request).unwrap_err();
    assert!(matches!(err, ChatFailure::Cancelled), "{err:?}");
    assert_eq!(asked(&client), 0);
}
