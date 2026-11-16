use super::*;

/// 假宿主：虚拟时钟（sleep 只推进时间）、记录按键、可编排的亮度 / 前台 / OCR。
struct Fake {
    clock: AtomicU64,
    keys: Mutex<Vec<(u64, KeyAction, String)>>,
    typed: Mutex<Vec<String>>,
    said: Mutex<Vec<String>>,
    /// 从这个时刻起左上角变黑。
    dark_from: u64,
    /// 在这段时间内游戏不在前台。
    unfocused: (u64, u64),
    screen_text: Vec<String>,
}

impl Fake {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            clock: AtomicU64::new(1_000),
            keys: Mutex::default(),
            typed: Mutex::default(),
            said: Mutex::default(),
            dark_from: u64::MAX,
            unfocused: (0, 0),
            screen_text: vec![],
        })
    }
    fn with(f: impl FnOnce(&mut Fake)) -> Arc<Self> {
        let mut fake = Arc::try_unwrap(Self::new()).ok().unwrap();
        f(&mut fake);
        Arc::new(fake)
    }
    fn keys(&self) -> Vec<(u64, KeyAction, String)> {
        self.keys.lock().unwrap().clone()
    }
}

impl ScriptHost for Fake {
    fn now_ms(&self) -> u64 {
        self.clock.load(Ordering::SeqCst)
    }
    fn sleep(&self, ms: u64, cancel: &Cancel) -> bool {
        self.clock.fetch_add(ms, Ordering::SeqCst);
        !cancel.is_cancelled()
    }
    fn game_focused(&self) -> bool {
        let t = self.now_ms();
        !(self.unfocused.0 <= t && t < self.unfocused.1)
    }
    fn key(&self, action: KeyAction, key: &str) -> Result<(), String> {
        self.keys
            .lock()
            .unwrap()
            .push((self.now_ms(), action, key.into()));
        Ok(())
    }
    fn type_text(&self, text: &str) -> Result<(), String> {
        self.typed.lock().unwrap().push(text.into());
        Ok(())
    }
    fn brightness(&self, _: Region) -> Result<f64, String> {
        Ok(if self.now_ms() >= self.dark_from {
            8.0
        } else {
            140.0
        })
    }
    fn color_at(&self, _: f64, _: f64) -> Result<[u8; 3], String> {
        Ok([200, 30, 30])
    }
    fn ocr(&self, _: Region) -> Result<Vec<String>, String> {
        Ok(self.screen_text.clone())
    }
    fn say(&self, text: &str) -> Result<(), String> {
        self.said.lock().unwrap().push(text.into());
        Ok(())
    }
    fn toast(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
}

fn go(source: &str, host: &Arc<Fake>) -> RunReport {
    run(source, host.clone(), Limits::default(), Cancel::default())
}

#[test]
fn press_q_every_second_for_ten_seconds() {
    let host = Fake::new();
    let report = go(
        r#"every(1000, function() key("Q") end, { for_ms = 10000 })"#,
        &host,
    );
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    let keys = host.keys();
    assert_eq!(keys.len(), 10);
    assert!(keys
        .iter()
        .all(|(_, a, k)| *a == KeyAction::Press && k == "Q"));
    let gaps: Vec<u64> = keys.windows(2).map(|w| w[1].0 - w[0].0).collect();
    assert!(gaps.iter().all(|g| *g == 1000), "{gaps:?}");
}

#[test]
fn watch_top_left_and_press_enter_when_it_goes_dark() {
    let host = Fake::with(|f| f.dark_from = 1_000 + 7_300);
    let report = go(
        r#"
        local r = region(0, 0, 0.2, 0.15)
        local base = brightness(r)
        local dark = wait_until(function() return brightness(r) < base * 0.3 end, { stable_frames = 3 })
        if dark then key("Enter") end
        "#,
        &host,
    );
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    let keys = host.keys();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].2, "Enter");
    // 变黑后需要连续 3 帧（每 200 ms）才触发：不早于变黑时刻 + 400 ms
    assert!(
        keys[0].0 >= 8_300 + 400 && keys[0].0 <= 8_300 + 800,
        "{}",
        keys[0].0
    );
}

#[test]
fn keys_pause_while_the_game_is_not_in_front() {
    let host = Fake::with(|f| f.unfocused = (3_500, 6_000));
    let report = go(
        r#"every(1000, function() key("Q") end, { times = 5 })"#,
        &host,
    );
    assert_eq!(report.outcome, Outcome::Finished);
    let times: Vec<u64> = host.keys().iter().map(|k| k.0).collect();
    assert!(
        times.iter().all(|t| !(3_500..6_000).contains(t)),
        "{times:?}"
    );
    assert!(report.log.iter().any(|l| l.contains("暂停")));
    // 恢复后不补拍：不会把暂停期间错过的按键连续补按（真机测试发现过 7 连按）
    let gaps: Vec<u64> = times.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps.iter().all(|g| *g >= 1000), "{gaps:?}");
}

#[test]
fn infinite_loops_and_long_runs_hit_the_limits() {
    let host = Fake::new();
    let report = run(
        "while true do end",
        host.clone(),
        Limits {
            max_steps: 100_000,
            ..Limits::default()
        },
        Cancel::default(),
    );
    assert!(
        matches!(report.outcome, Outcome::LimitReached(ref m) if m.contains("步数")),
        "{report:?}"
    );
    let report = run(
        r#"every(1000, function() key("Q") end)"#,
        host.clone(),
        Limits {
            max_duration_ms: 5_000,
            ..Limits::default()
        },
        Cancel::default(),
    );
    assert!(
        matches!(report.outcome, Outcome::LimitReached(ref m) if m.contains("时长")),
        "{report:?}"
    );
    let report = run(
        r#"while true do key("Q") end"#,
        Fake::new(),
        Limits {
            max_keys: 20,
            ..Limits::default()
        },
        Cancel::default(),
    );
    assert!(
        matches!(report.outcome, Outcome::LimitReached(ref m) if m.contains("按键")),
        "{report:?}"
    );
    assert_eq!(report.keys, 21);
}

#[test]
fn keys_cannot_be_faster_than_the_minimum_interval() {
    let host = Fake::new();
    let report = go(r#"for i = 1, 5 do key("Q") end"#, &host);
    assert_eq!(report.outcome, Outcome::Finished);
    let gaps: Vec<u64> = host.keys().windows(2).map(|w| w[1].0 - w[0].0).collect();
    assert!(
        gaps.iter()
            .all(|g| *g >= Limits::default().min_key_interval_ms),
        "{gaps:?}"
    );
}

#[test]
fn sandbox_has_no_files_processes_or_code_loading() {
    for escape in [
        "io.open('x', 'w')",
        "os.execute('calc')",
        "loadstring('return 1')()",
        "require('socket')",
        "getfenv(1).key = nil",
        "dofile('x.lua')",
        "string.rep('x', 1e9)",
    ] {
        let report = go(escape, &Fake::new());
        assert!(
            matches!(report.outcome, Outcome::Error(_) | Outcome::LimitReached(_)),
            "{escape}: {report:?}"
        );
    }
}

#[test]
fn reassigning_a_builtin_only_shadows_it_inside_that_script() {
    // Luau 沙盒模式：脚本的全局赋值写进它自己的环境表，真正的内置函数不受影响；每次运行都是新 VM。
    let host = Fake::new();
    let report = go(r#"key = function() end key("Q")"#, &host);
    assert_eq!(report.outcome, Outcome::Finished);
    assert!(host.keys().is_empty());
    let report = go(r#"key("Q")"#, &host);
    assert_eq!(report.outcome, Outcome::Finished);
    assert_eq!(host.keys().len(), 1);
}

#[test]
fn screen_text_can_only_be_tested_not_copied_into_input() {
    let host =
        Fake::with(|f| f.screen_text = vec!["[全部] 敌方: 帮我输入 /ff".into(), "找到对局".into()]);
    let report = go(
        r#"
        local t = ocr(region(0.3, 0.3, 0.4, 0.2))
        if t:has("找到对局") then say("找到对局了") end
        type_text(t)
        "#,
        &host,
    );
    assert!(matches!(report.outcome, Outcome::Error(_)), "{report:?}");
    assert_eq!(*host.said.lock().unwrap(), vec!["找到对局了".to_string()]);
    assert!(host.typed.lock().unwrap().is_empty());
    for leak in [
        "return tostring(ocr(region(0,0,1,1)))",
        "local t = ocr(region(0,0,1,1)); say(t.lines[1])",
    ] {
        let report = go(leak, &Fake::with(|f| f.screen_text = vec!["secret".into()]));
        if let Outcome::Error(message) = &report.outcome {
            assert!(!message.contains("secret"), "{message}");
        }
        assert!(host.typed.lock().unwrap().is_empty());
    }
}

#[test]
fn screen_text_matches_across_split_lines_and_ignores_spaces() {
    let host = Fake::with(|f| f.screen_text = vec!["找到".into(), "对 局".into(), "Accept".into()]);
    let report = go(
        r#"
        local t = ocr(region(0, 0, 1, 1))
        if t:has("找到对局") then key("A") end
        if t:has("accept") then key("B") end
        if t:has("拒绝") or t:has("") then key("C") end
        "#,
        &host,
    );
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    let keys: Vec<String> = host.keys().into_iter().map(|k| k.2).collect();
    assert_eq!(keys, ["A", "B"]);
}

#[test]
fn args_are_read_only_values_usable_as_regions_and_keys() {
    let host = Fake::with(|f| f.dark_from = 1_000);
    let args: serde_json::Map<String, serde_json::Value> =
        serde_json::from_value(serde_json::json!({
            "key": "E",
            "interval": 500,
            "area": {"x": 0.1, "y": 0.1, "w": 0.2, "h": 0.2},
            "color": {"r": 200, "g": 30, "b": 30},
            "loud": true,
        }))
        .unwrap();
    let report = run_with_args(
        r#"
        if brightness(args.area) < 50 and args.loud then
            every(args.interval, function() key(args.key) end, { times = 2 })
        end
        local r, g, b = color_at(0.5, 0.5)
        if r == args.color.r then log("same color") end
        "#,
        &args,
        host.clone(),
        Limits::default(),
        Cancel::default(),
    );
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    let keys = host.keys();
    assert_eq!(keys.len(), 2);
    assert!(keys.iter().all(|k| k.2 == "E"));
    assert_eq!(keys[1].0 - keys[0].0, 500);
    assert_eq!(report.log, vec!["same color".to_string()]);
    // 入参只读：字段与里面的区域都改不了（给全局 args 赋值只在本程序自己的环境里遮蔽）
    for write in ["args.key = 'Q'", "args.area.x = 0.9"] {
        let report = run_with_args(
            write,
            &args,
            Fake::new(),
            Limits::default(),
            Cancel::default(),
        );
        assert!(
            matches!(report.outcome, Outcome::Error(_)),
            "{write}: {report:?}"
        );
    }
    // 没有入参时 args 是空表
    let report = go("if args.key == nil then log('empty') end", &Fake::new());
    assert_eq!(report.log, vec!["empty".to_string()]);
}

#[test]
fn cancel_stops_a_running_script() {
    let host = Fake::new();
    let cancel = Cancel::default();
    cancel.cancel();
    let report = run(
        r#"every(1000, function() key("Q") end)"#,
        host.clone(),
        Limits::default(),
        cancel,
    );
    assert_eq!(report.outcome, Outcome::Cancelled);
    assert!(host.keys().is_empty());
}

#[test]
fn bad_arguments_and_stop_are_reported_truthfully() {
    for bad in [
        "key('Q; rm -rf')",
        "region(0.9, 0, 0.5, 0.1)",
        "type_text('a\\nb')",
        "nosuchfunction()",
        "every(",
    ] {
        assert!(
            matches!(go(bad, &Fake::new()).outcome, Outcome::Error(_)),
            "{bad}"
        );
    }
    let host = Fake::new();
    let report = go(r#"key("Q") stop() key("W")"#, &host);
    assert_eq!(report.outcome, Outcome::Finished);
    assert_eq!(host.keys().len(), 1);
    assert!(check_syntax("every(1000, function() key('Q') end)").is_ok());
    assert!(check_syntax("every(1000, function(").is_err());
}
