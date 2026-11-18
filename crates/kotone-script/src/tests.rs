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
    /// 带位置的识别结果（设置了就优先于 `screen_text`）。
    text_lines: Vec<TextLine>,
    mouse: Mutex<Vec<(u64, MouseAction)>>,
}

/// 100×50 的测试画面：白底；(10..15, 5..10) 一小块红、(60..80, 20..30) 一大块红；
/// (90, 40) 一个蓝点；(30..34, 30..34) 一个 4×4 的绿框（中心白）。
fn scene() -> Pixels {
    let (w, h) = (100u32, 50u32);
    let mut rgba = vec![255u8; (w * h * 4) as usize];
    let mut put = |x: u32, y: u32, c: [u8; 3]| {
        let i = ((y * w + x) * 4) as usize;
        rgba[i..i + 3].copy_from_slice(&c);
    };
    for y in 5..10 {
        for x in 10..15 {
            put(x, y, [200, 30, 30]);
        }
    }
    for y in 20..30 {
        for x in 60..80 {
            put(x, y, [204, 28, 33]);
        }
    }
    put(90, 40, [20, 40, 220]);
    for y in 30..34 {
        for x in 30..34 {
            if !(31..33).contains(&x) || !(31..33).contains(&y) {
                put(x, y, [0, 160, 0]);
            }
        }
    }
    Pixels {
        width: w,
        height: h,
        rgba,
        origin: Region {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
    }
}

/// 从测试画面裁一块当模板。
fn crop(x0: u32, y0: u32, w: u32, h: u32) -> Pixels {
    let full = scene();
    let mut rgba = Vec::new();
    for y in y0..y0 + h {
        let i = ((y * full.width + x0) * 4) as usize;
        rgba.extend_from_slice(&full.rgba[i..i + (w * 4) as usize]);
    }
    Pixels {
        width: w,
        height: h,
        rgba,
        origin: full.origin,
    }
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
            text_lines: vec![],
            mouse: Mutex::default(),
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
    fn pixels(&self, region: Region) -> Result<Pixels, String> {
        let mut pixels = scene();
        pixels.origin = region;
        Ok(pixels)
    }
    fn ocr(&self, _: Region) -> Result<Vec<TextLine>, String> {
        if !self.text_lines.is_empty() {
            return Ok(self.text_lines.clone());
        }
        Ok(self
            .screen_text
            .iter()
            .map(|t| TextLine {
                text: t.clone(),
                rect: None,
            })
            .collect())
    }
    fn image(&self, id: &str) -> Result<Pixels, String> {
        match id {
            "red-corner" => Ok(crop(58, 18, 6, 6)),
            "green-ring" => Ok(crop(30, 30, 4, 4)),
            _ => Err(format!("没有图片 {id}")),
        }
    }
    fn mouse(&self, action: MouseAction) -> Result<(), String> {
        self.mouse.lock().unwrap().push((self.now_ms(), action));
        Ok(())
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

fn line(text: &str, x: f64, y: f64, w: f64, h: f64) -> TextLine {
    TextLine {
        text: text.into(),
        rect: Some(Region {
            x,
            y,
            width: w,
            height: h,
        }),
    }
}

fn logged(source: &str, host: &Arc<Fake>) -> Vec<String> {
    let report = go(source, host);
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    report.log
}

#[test]
fn find_color_returns_the_first_match_like_pixel_search_or_the_largest_patch() {
    let host = Fake::new();
    let log = logged(
        r##"
        local all = region(0, 0, 1, 1)
        local x, y, n = find_color(all, {r = 200, g = 30, b = 30})
        log(string.format("first %.3f %.3f %d", x, y, n))
        x, y = find_color(all, {r = 200, g = 30, b = 30}, { variation = 5, largest = true })
        log(string.format("largest %.3f %.3f", x, y))
        x = find_color(all, "#CC1C21")
        log(string.format("hex %.3f", x))
        if find_color(all, {r = 1, g = 2, b = 3}) == nil then log("none") end
        "##,
        &host,
    );
    // 第一个红像素是小块的左上角 (10, 5) 的像素中心；完全匹配时大块（204,28,33）不算
    assert_eq!(log[0], "first 0.105 0.110 1");
    // 容差 5 时大块也匹配，最大一块是 (60..80, 20..30)，中心 (70, 25)
    assert_eq!(log[1], "largest 0.700 0.500");
    assert_eq!(log[2], "hex 0.605");
    assert_eq!(log[3], "none");
    let report = go(
        r##"find_color(region(0, 0, 1, 1), "#14_8DC")"##,
        &Fake::new(),
    );
    assert!(matches!(report.outcome, Outcome::Error(ref m) if m.contains("颜色")));
    // 区域换算：只看右半边时坐标仍是整个窗口的比例
    let log = logged(
        r#"local x = find_color(region(0.5, 0, 0.5, 1), {r = 20, g = 40, b = 220}) log(string.format("%.3f", x))"#,
        &Fake::new(),
    );
    assert_eq!(log[0], format!("{:.3}", 0.5 + 90.5 / 100.0 * 0.5));
}

#[test]
fn find_image_matches_a_template_with_variation_and_transparency() {
    let log = logged(
        r#"
        local all = region(0, 0, 1, 1)
        local cx, cy, lx, ly = find_image(all, { image = "red-corner" })
        log(string.format("tl %.2f %.2f c %.2f %.2f", lx, ly, cx, cy))
        cx, cy, lx, ly = find_image(all, { image = "red-corner" }, { variation = 8 })
        log(string.format("loose %.2f %.2f", lx, ly))
        cx, cy, lx, ly = find_image(all, { image = "green-ring" }, { trans = {r = 255, g = 255, b = 255} })
        log(string.format("ring %.2f %.2f", lx, ly))
        "#,
        &Fake::new(),
    );
    // 模板取自大块 (204,28,33) 的左上角（带两格白边）：完全匹配只在那里
    assert_eq!(log[0], "tl 0.58 0.36 c 0.61 0.42");
    // 容差 8 时小块 (200,30,30) 也算匹配，按“第一个匹配”先找到更靠上的小块
    assert_eq!(log[1], "loose 0.08 0.06");
    // 透明色（白）不参与比较：只比较绿框本身
    assert_eq!(log[2], "ring 0.30 0.60");
    for bad in [
        r#"find_image(region(0,0,1,1), { image = "missing" })"#,
        r#"find_image(region(0,0,1,1), { png = "abc" })"#,
    ] {
        assert!(
            matches!(go(bad, &Fake::new()).outcome, Outcome::Error(_)),
            "{bad}"
        );
    }
}

#[test]
fn find_text_returns_where_the_text_is() {
    let host = Fake::with(|f| {
        f.text_lines = vec![
            line("[全部] 敌方: 投降吧", 0.1, 0.8, 0.4, 0.05),
            line("找到", 0.40, 0.40, 0.10, 0.06),
            line("对局", 0.52, 0.40, 0.10, 0.06),
            line("接受 拒绝", 0.40, 0.60, 0.20, 0.06),
        ]
    });
    let log = logged(
        r#"
        local all = region(0, 0, 1, 1)
        local x, y = find_text(all, "找到对局")
        log(string.format("split %.2f %.2f", x, y))
        x, y = find_text(all, "拒绝")
        log(string.format("part %.2f %.2f", x, y))
        if find_text(all, "开始") == nil then log("none") end
        local t = ocr(all)
        x, y = t:find("接受")
        log(string.format("method %.2f", x))
        if t:has("投降") then log("has") end
        "#,
        &host,
    );
    // 跨两行：取两行外接框 (0.40..0.62) 的中心
    assert_eq!(log[0], "split 0.51 0.43");
    // 一行里的后半段：“接受拒绝”四个字里的第 3–4 个，中心在 0.40 + 0.20 × 3/4
    assert_eq!(log[1], "part 0.55 0.63");
    assert_eq!(log[2], "none");
    assert_eq!(log[3], "method 0.45");
    assert_eq!(log[4], "has");
    // 模型不给位置时如实报错
    let host = Fake::with(|f| f.screen_text = vec!["找到对局".into()]);
    let report = go(r#"find_text(region(0,0,1,1), "找到")"#, &host);
    assert!(matches!(report.outcome, Outcome::Error(ref m) if m.contains("位置")));
}

#[test]
fn mouse_actions_use_window_ratios_and_share_the_input_guards() {
    let host = Fake::with(|f| f.unfocused = (1_200, 2_000));
    let report = go(
        r#"
        click(0.5, 0.25)
        click(0.1, 0.9, { button = "right", count = 2 })
        mouse_move(0.2, 0.2) mouse_down() mouse_move(0.8, 0.2) mouse_up()
        scroll(-3)
        "#,
        &host,
    );
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    let actions: Vec<MouseAction> = host.mouse.lock().unwrap().iter().map(|m| m.1).collect();
    assert_eq!(
        actions,
        vec![
            MouseAction::Click {
                x: 0.5,
                y: 0.25,
                button: MouseButton::Left,
                count: 1
            },
            MouseAction::Click {
                x: 0.1,
                y: 0.9,
                button: MouseButton::Right,
                count: 2
            },
            MouseAction::Move { x: 0.2, y: 0.2 },
            MouseAction::Down(MouseButton::Left),
            MouseAction::Move { x: 0.8, y: 0.2 },
            MouseAction::Up(MouseButton::Left),
            MouseAction::Scroll(-3),
        ]
    );
    // 与按键共用：最小间隔、失焦暂停、计数
    // mouse_up 与 key_up 一样随时可以发（不等间隔、不等前台），不在检查范围内
    let times: Vec<u64> = host
        .mouse
        .lock()
        .unwrap()
        .iter()
        .filter(|m| !matches!(m.1, MouseAction::Up(_)))
        .map(|m| m.0)
        .collect();
    assert!(times.windows(2).all(|w| w[1] - w[0] >= 50), "{times:?}");
    assert!(
        times.iter().all(|t| !(1_200..2_000).contains(t)),
        "{times:?}"
    );
    assert_eq!(report.keys, 6, "mouse_up 不计数");
    for bad in [
        "click(1.2, 0.5)",
        "click(0.5, 0.5, { button = 'back' })",
        "click(0.5, 0.5, { count = 5 })",
        "scroll(0)",
        "scroll(50)",
        "mouse_move(0/0, 0.5)",
    ] {
        assert!(
            matches!(go(bad, &Fake::new()).outcome, Outcome::Error(_)),
            "{bad}"
        );
    }
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

// ---------- 内置的常用按键程序（kotone-core 自动化库，ADR 013） ----------

fn builtin(id: &str) -> (String, serde_json::Map<String, serde_json::Value>) {
    use kotone_core::automation::library::{validate_args, AutomationBody, AutomationLibrary};
    let a = AutomationLibrary::builtin_programs_preview()
        .into_iter()
        .find(|a| a.id == id)
        .unwrap();
    let AutomationBody::Program(p) = a.action else {
        panic!("{id} 不是程序")
    };
    let args = validate_args(&a.params, &serde_json::Map::new()).unwrap();
    (p.source, args)
}

fn go_builtin(id: &str, host: &Arc<Fake>, overrides: serde_json::Value) -> RunReport {
    let (source, mut args) = builtin(id);
    if let serde_json::Value::Object(o) = overrides {
        args.extend(o);
    }
    run_with_args(
        &source,
        &args,
        host.clone(),
        Limits::default(),
        Cancel::default(),
    )
}

#[test]
fn every_builtin_program_compiles() {
    for a in kotone_core::automation::library::AutomationLibrary::builtin_programs_preview() {
        let kotone_core::automation::library::AutomationBody::Program(p) = &a.action else {
            continue;
        };
        check_syntax(&p.source).unwrap_or_else(|e| panic!("{}: {e}", a.id));
    }
}

#[test]
fn builtin_key_repeat_presses_on_the_beat() {
    let host = Fake::new();
    let report = go_builtin(
        "builtin-key-repeat",
        &host,
        serde_json::json!({"key": "F", "interval_ms": 500, "seconds": 5}),
    );
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    let keys = host.keys();
    assert_eq!(keys.len(), 10);
    assert!(keys
        .iter()
        .all(|(_, a, k)| *a == KeyAction::Press && k == "F"));
    assert!(keys.windows(2).all(|w| w[1].0 - w[0].0 == 500));
}

#[test]
fn builtin_auto_click_hold_and_sequence() {
    let host = Fake::new();
    let report = go_builtin(
        "builtin-auto-click",
        &host,
        serde_json::json!({"seconds": 1, "button": "右键"}),
    );
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    let clicks = host.mouse.lock().unwrap().clone();
    assert_eq!(clicks.len(), 5, "200 ms 一下，点 1 秒");
    assert!(
        matches!(clicks[0].1, MouseAction::Click { button: MouseButton::Right, x, y, .. } if (x - 0.5).abs() < 1e-9 && (y - 0.5).abs() < 1e-9)
    );

    let host = Fake::new();
    go_builtin(
        "builtin-hold-key",
        &host,
        serde_json::json!({"key": "W", "seconds": 3}),
    );
    let keys = host.keys();
    assert_eq!(keys.len(), 2);
    assert_eq!((keys[0].1, keys[1].1), (KeyAction::Down, KeyAction::Up));
    assert_eq!(keys[1].0 - keys[0].0, 3000);

    let host = Fake::new();
    go_builtin(
        "builtin-key-sequence",
        &host,
        serde_json::json!({"keys": "Q  W E", "times": 2}),
    );
    let pressed: Vec<String> = host.keys().into_iter().map(|(_, _, k)| k).collect();
    assert_eq!(pressed, ["Q", "W", "E", "Q", "W", "E"]);
}

#[test]
fn builtin_chat_repeat_opens_types_and_sends() {
    let host = Fake::new();
    let report = go_builtin(
        "builtin-chat-repeat",
        &host,
        serde_json::json!({"text": "来人", "times": 2, "interval_seconds": 5}),
    );
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    assert_eq!(host.typed.lock().unwrap().clone(), ["来人", "来人"]);
    let keys: Vec<String> = host.keys().into_iter().map(|(_, _, k)| k).collect();
    assert_eq!(keys, ["Enter", "Enter", "Enter", "Enter"]);
}

#[test]
fn builtin_text_and_color_watchers() {
    // 看到“接受”就点它
    let host = Fake::with(|f| {
        f.text_lines = vec![
            line("找到对局", 0.40, 0.40, 0.20, 0.06),
            line("接受", 0.45, 0.70, 0.10, 0.05),
        ]
    });
    let report = go_builtin(
        "builtin-text-then-key",
        &host,
        serde_json::json!({"minutes": 1}),
    );
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    assert_eq!(host.mouse.lock().unwrap().len(), 1, "点了一下就结束");
    // 没等到：不点也不按
    let host = Fake::new();
    go_builtin(
        "builtin-text-then-key",
        &host,
        serde_json::json!({"minutes": 1, "text": "不存在"}),
    );
    assert!(host.mouse.lock().unwrap().is_empty() && host.keys().is_empty());
    // 测试画面右侧有一大块红：“颜色出现时”每次看都按，按完歇 3 秒
    let host = Fake::new();
    let report = go_builtin(
        "builtin-color-then-key",
        &host,
        serde_json::json!({"area": {"x": 0.5, "y": 0.3, "w": 0.4, "h": 0.4}, "color": {"r": 200, "g": 30, "b": 30},
                           "when": "颜色出现时", "minutes": 1, "variation": 10}),
    );
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    let n = host.keys().len();
    assert!((10..=20).contains(&n), "{n}");
    // “颜色消失时”：红色一直在，就一直不按
    let host = Fake::new();
    go_builtin(
        "builtin-color-then-key",
        &host,
        serde_json::json!({"area": {"x": 0.5, "y": 0.3, "w": 0.4, "h": 0.4}, "color": {"r": 200, "g": 30, "b": 30}, "minutes": 1, "variation": 10}),
    );
    assert!(host.keys().is_empty());
}
