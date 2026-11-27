use super::*;

/// 假宿主：虚拟时钟（sleep 只推进时间）、记录按键、可编排的亮度 / 前台 / OCR。
struct Fake {
    clock: AtomicU64,
    keys: Mutex<Vec<(u64, KeyAction, String)>>,
    typed: Mutex<Vec<String>>,
    /// `chat()`：(时刻, 文字, 频道)
    chatted: Mutex<Vec<(u64, String, Option<String>)>>,
    /// `say()`：(文字, 同时弹幕)
    said: Mutex<Vec<(String, bool)>>,
    toasted: Mutex<Vec<String>>,
    /// `push()`：(时刻, 文字)
    pushed: Mutex<Vec<(u64, String)>>,
    asked: Mutex<Vec<String>>,
    /// 这次执行绑没绑游戏窗口（ADR 045 D4）。
    window: bool,
    /// 说话时被停下：先取消再报错（模拟宿主在停下时以错误返回）。
    interrupt: Option<Cancel>,
    /// 从这个时刻起左上角变黑。
    dark_from: u64,
    /// 在这段时间内游戏不在前台。
    unfocused: (u64, u64),
    screen_text: Vec<String>,
    /// 带位置的识别结果（设置了就优先于 `screen_text`）。
    text_lines: Vec<TextLine>,
    mouse: Mutex<Vec<(u64, MouseAction)>>,
    /// `tool()`：(开始时刻, 回来时刻, 名字, 参数)
    tool_calls: Mutex<Vec<(u64, u64, String, serde_json::Value)>>,
    /// 工具怎么回（没设 = 做成了，`{status = "ok"}`）。
    tool_reply: Option<ToolAnswer>,
    /// 一次工具调用要多久（推进虚拟时钟）。
    tool_ms: u64,
}

type ToolAnswer = Arc<dyn Fn(&str, &serde_json::Value) -> Result<ToolReply, String> + Send + Sync>;

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
            chatted: Mutex::default(),
            said: Mutex::default(),
            toasted: Mutex::default(),
            pushed: Mutex::default(),
            asked: Mutex::default(),
            window: true,
            interrupt: None,
            dark_from: u64::MAX,
            unfocused: (0, 0),
            screen_text: vec![],
            text_lines: vec![],
            mouse: Mutex::default(),
            tool_calls: Mutex::default(),
            tool_reply: None,
            tool_ms: 0,
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
    fn chat(&self, text: &str, channel: Option<&str>) -> Result<(), String> {
        self.chatted
            .lock()
            .unwrap()
            .push((self.now_ms(), text.into(), channel.map(String::from)));
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
    fn say(&self, text: &str, toast: bool) -> Result<(), String> {
        if let Some(cancel) = &self.interrupt {
            cancel.cancel();
            return Err("说到一半停下了".into());
        }
        self.said.lock().unwrap().push((text.into(), toast));
        Ok(())
    }
    fn toast(&self, text: &str) -> Result<(), String> {
        self.toasted.lock().unwrap().push(text.into());
        Ok(())
    }
    fn push(&self, text: &str) -> Result<(), String> {
        self.pushed
            .lock()
            .unwrap()
            .push((self.now_ms(), text.into()));
        Ok(())
    }
    fn ask(&self, text: &str) -> Result<bool, String> {
        self.asked.lock().unwrap().push(text.into());
        Ok(true)
    }
    fn require_window(&self) -> Result<(), String> {
        if self.window {
            Ok(())
        } else {
            Err("这段程序没有绑定游戏窗口".into())
        }
    }
    fn tool(&self, name: &str, args: &serde_json::Value) -> Result<ToolReply, String> {
        let started = self.now_ms();
        self.clock.fetch_add(self.tool_ms, Ordering::SeqCst);
        self.tool_calls
            .lock()
            .unwrap()
            .push((started, self.now_ms(), name.into(), args.clone()));
        if let Some(cancel) = &self.interrupt {
            cancel.cancel();
            return Err("等工具的时候停下了".into());
        }
        match &self.tool_reply {
            Some(reply) => reply(name, args),
            None => Ok(ToolReply::Value(serde_json::json!({"status": "ok"}))),
        }
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
    assert_eq!(
        *host.said.lock().unwrap(),
        vec![("找到对局了".to_string(), false)]
    );
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

/// 内置自动化的源码，和按声明校验过的入参（`given` 之外的用缺省值）。
fn builtin(
    id: &str,
    given: &serde_json::Map<String, serde_json::Value>,
) -> (String, serde_json::Map<String, serde_json::Value>) {
    use kotone_core::automation::library::{validate_args, AutomationLibrary};
    let a = AutomationLibrary::builtin_programs_preview()
        .into_iter()
        .find(|a| a.id == id)
        .unwrap();
    let args = validate_args(&a.params, given).unwrap();
    (a.action.source().to_string(), args)
}

fn go_builtin(id: &str, host: &Arc<Fake>, overrides: serde_json::Value) -> RunReport {
    let serde_json::Value::Object(given) = overrides else {
        panic!("overrides 要是对象")
    };
    let (source, args) = builtin(id, &given);
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
        check_syntax(a.action.source()).unwrap_or_else(|e| panic!("{}: {e}", a.id));
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

/// 定时喊话经本体的“发到游戏”入口（ADR 041 D1）：不再自己按回车（聊天键归游戏配置，CS2 是 U）。
#[test]
fn builtin_chat_repeat_sends_through_the_entry() {
    let host = Fake::new();
    let report = go_builtin(
        "builtin-chat-repeat",
        &host,
        serde_json::json!({"text": "来人", "times": 2, "interval_seconds": 5}),
    );
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    let chatted = host.chatted.lock().unwrap().clone();
    assert_eq!(chatted.len(), 2);
    assert!(chatted.iter().all(|(_, t, c)| t == "来人" && c.is_none()));
    assert_eq!(chatted[1].0 - chatted[0].0, 5000);
    assert!(host.keys().is_empty() && host.typed.lock().unwrap().is_empty());
}

#[test]
fn chat_counts_as_an_input_and_checks_its_arguments() {
    let host = Fake::new();
    let report = go(
        r#"chat("gg", { channel = "all" }) chat("第一行\n第二行")"#,
        &host,
    );
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    assert_eq!(report.keys, 2);
    let chatted = host.chatted.lock().unwrap().clone();
    assert_eq!(chatted[0].1, "gg");
    assert_eq!(chatted[0].2.as_deref(), Some("all"));
    assert_eq!(chatted[1].1, "第一行\n第二行", "分行交给入口");
    assert!(chatted[1].0 - chatted[0].0 >= Limits::default().min_key_interval_ms);
    for bad in [
        "chat('')",
        "chat('gg', { channel = 'party' })",
        "chat(ocr(region(0, 0, 1, 1)))",
    ] {
        assert!(
            matches!(go(bad, &Fake::new()).outcome, Outcome::Error(_)),
            "{bad}"
        );
    }
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

// ---------- ADR 045：通知、交给她都是函数；用不用游戏窗口看程序里写了什么 ----------

/// 一个装好白名单函数的虚拟机里，多出来的全局名字。
fn whitelist() -> Vec<String> {
    let libs = StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8;
    let names = |lua: &Lua| -> Vec<String> {
        lua.globals()
            .pairs::<String, Value>()
            .filter_map(|pair| pair.ok())
            .filter(|(_, v)| matches!(v, Value::Function(_)))
            .map(|(k, _)| k)
            .collect()
    };
    let bare = Lua::new_with(libs, LuaOptions::default()).unwrap();
    let before = names(&bare);
    let lua = Lua::new_with(libs, LuaOptions::default()).unwrap();
    let host: Arc<dyn ScriptHost> = Fake::new();
    install(&lua, State::new(host, Limits::default(), Cancel::default())).unwrap();
    let mut added: Vec<String> = names(&lua)
        .into_iter()
        .filter(|n| !before.contains(n))
        .collect();
    added.sort();
    added
}

/// 白名单里每个函数都被 core 的两张表归了类（用不用游戏窗口），表里也没有多出来的名字。
#[test]
fn every_whitelisted_function_is_classified_by_core() {
    use kotone_core::automation::library::{PLAIN_FUNCTIONS, WINDOW_FUNCTIONS};
    let installed = whitelist();
    assert!(installed.len() > 20, "{installed:?}");
    for name in &installed {
        let window = WINDOW_FUNCTIONS.contains(&name.as_str());
        let plain = PLAIN_FUNCTIONS.contains(&name.as_str());
        assert!(
            window ^ plain,
            "{name} 要在 WINDOW_FUNCTIONS / PLAIN_FUNCTIONS 之一里"
        );
    }
    for name in WINDOW_FUNCTIONS.iter().chain(PLAIN_FUNCTIONS) {
        assert!(installed.iter().any(|n| n == name), "{name} 不在白名单里");
    }
}

/// 没绑窗口：窗口函数一调用就报错，不进“游戏不在前台就暂停”的等待；`game_focused()` 是 false；说话照常。
#[test]
fn without_a_window_window_functions_fail_at_once() {
    let unbound = || {
        Fake::with(|f| {
            f.window = false;
            // 真进了暂停循环就会一直等到总时长上限
            f.unfocused = (0, u64::MAX);
        })
    };
    for source in [
        "key('Q')",
        "key_up('Q')",
        "type_text('gg')",
        "chat('gg')",
        "click(0.5, 0.5)",
        "mouse_up()",
        "scroll(1)",
        "brightness(region(0, 0, 1, 1))",
        "color_at(0.5, 0.5)",
        "ocr(region(0, 0, 1, 1))",
        "find_text(region(0, 0, 1, 1), 'x')",
        "find_color(region(0, 0, 1, 1), {r = 1, g = 2, b = 3})",
        "find_image(region(0, 0, 1, 1), { image = 'red-corner' })",
    ] {
        let host = unbound();
        let report = go(source, &host);
        assert!(
            matches!(&report.outcome, Outcome::Error(m) if m.contains("没有绑定游戏窗口")),
            "{source}: {report:?}"
        );
        assert!(report.duration_ms < 1_000, "{source}: 不等");
        assert!(host.keys().is_empty() && host.mouse.lock().unwrap().is_empty());
        assert!(!report.log.iter().any(|l| l.contains("暂停")), "{source}");
    }
    let host = unbound();
    let report = go(
        "if not game_focused() then say('不在前台') end toast('好') push('记一下') tool('music')",
        &host,
    );
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    assert_eq!(
        *host.said.lock().unwrap(),
        vec![("不在前台".to_string(), false)]
    );
    assert_eq!(*host.toasted.lock().unwrap(), vec!["好".to_string()]);
    // 调工具不算用到游戏窗口（ADR 046 D1）
    assert_eq!(host.tool_calls.lock().unwrap().len(), 1);
    // 绑了窗口时照旧
    let report = go("if game_focused() then log('在') end", &Fake::new());
    assert_eq!(report.log, vec!["在".to_string()]);
}

#[test]
fn say_toast_push_and_ask_check_their_text() {
    let host = Fake::new();
    let report = go(
        r#"
        say("  该买眼了 ")
        say("小龙要刷了", { toast = true })
        toast("弹一下")
        push("买了真眼")
        if ask("看看对面谁是突破口") then log("交出去了") end
        "#,
        &host,
    );
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    assert_eq!(
        *host.said.lock().unwrap(),
        vec![
            ("该买眼了".to_string(), false),
            ("小龙要刷了".to_string(), true)
        ]
    );
    assert_eq!(*host.toasted.lock().unwrap(), vec!["弹一下".to_string()]);
    assert_eq!(host.pushed.lock().unwrap()[0].1, "买了真眼");
    assert_eq!(
        *host.asked.lock().unwrap(),
        vec!["看看对面谁是突破口".to_string()]
    );
    assert_eq!(report.log, vec!["交出去了".to_string()]);
    let ok = |source: String| go(&source, &Fake::new()).outcome == Outcome::Finished;
    assert!(ok(format!("say('{}')", "说".repeat(MAX_SAY_CHARS))));
    assert!(!ok(format!("say('{}')", "说".repeat(MAX_SAY_CHARS + 1))));
    assert!(ok(format!("toast('{}')", "弹".repeat(MAX_SAY_CHARS))));
    assert!(!ok(format!("toast('{}')", "弹".repeat(MAX_SAY_CHARS + 1))));
    assert!(ok(format!("push('{}')", "推".repeat(MAX_PUSH_CHARS))));
    assert!(!ok(format!("push('{}')", "推".repeat(MAX_PUSH_CHARS + 1))));
    assert!(ok(format!("ask('{}')", "问".repeat(MAX_ASK_CHARS))));
    assert!(!ok(format!("ask('{}')", "问".repeat(MAX_ASK_CHARS + 1))));
    for bad in [
        "say('')",
        "toast('  ')",
        "push('a\\nb')",
        "ask('')",
        "say(ocr(region(0, 0, 1, 1)))",
    ] {
        assert!(!ok(bad.to_string()), "{bad}");
    }
}

/// 两次推送至少隔 1 秒：太快就等，不报错。
#[test]
fn pushes_are_spaced_at_least_a_second_apart() {
    let host = Fake::new();
    let report = go("for i = 1, 3 do push('第' .. i .. '条') end", &host);
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    let pushed = host.pushed.lock().unwrap().clone();
    assert_eq!(pushed.len(), 3);
    assert!(
        pushed
            .windows(2)
            .all(|w| w[1].0 - w[0].0 >= MIN_PUSH_INTERVAL_MS),
        "{pushed:?}"
    );
}

/// 停下时宿主以错误返回（正在说的话被打断）：记成“已取消”，不是程序出错。
#[test]
fn a_host_error_after_cancel_is_a_cancel() {
    let cancel = Cancel::default();
    let host = Fake::with(|f| f.interrupt = Some(cancel.clone()));
    let report = run("say('说一句')", host, Limits::default(), cancel);
    assert_eq!(report.outcome, Outcome::Cancelled, "{report:?}");
    // 没停下时宿主报错照旧是错误
    let report = go(
        "find_image(region(0,0,1,1), { image = 'missing' })",
        &Fake::new(),
    );
    assert!(matches!(report.outcome, Outcome::Error(ref m) if m.contains("missing")));
}

/// 新建时选的模板（kotone-core）：能编译，只填缺省值就能跑。
#[test]
fn every_template_compiles_and_runs_with_its_defaults() {
    use kotone_core::automation::library::{templates, validate_args};
    for t in templates() {
        check_syntax(&t.source).unwrap_or_else(|e| panic!("{}: {e}", t.id));
        let args = validate_args(&t.params, &serde_json::Map::new())
            .unwrap_or_else(|e| panic!("{}: {e}", t.id));
        // 模板都不用游戏窗口：没绑窗口也要能跑
        let host = Fake::with(|f| f.window = false);
        let report = run_with_args(
            &t.source,
            &args,
            host.clone(),
            Limits::default(),
            Cancel::default(),
        );
        assert_eq!(report.outcome, Outcome::Finished, "{}: {report:?}", t.id);
        let did = (
            host.said.lock().unwrap().len(),
            host.pushed.lock().unwrap().len(),
            host.asked.lock().unwrap().len(),
        );
        let want = match t.id.as_str() {
            "say" => (1, 0, 0),
            "push" | "onedragon-push" => (0, 1, 0),
            "ask" => (0, 0, 1),
            _ => (0, 0, 0),
        };
        assert_eq!(did, want, "{}", t.id);
    }
}

/// 内置的“X 秒之后提醒我”和突破口新装时就是程序（ADR 045 D1）：不用窗口也能跑。
#[test]
fn builtin_reminder_and_scout_run_without_a_window() {
    let host = Fake::with(|f| f.window = false);
    let report = go_builtin(
        "builtin-remind-later",
        &host,
        serde_json::json!({"text": "打小龙"}),
    );
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    assert_eq!(
        *host.said.lock().unwrap(),
        vec![("打小龙".to_string(), true)],
        "语音和弹幕一次投递"
    );
    let host = Fake::with(|f| f.window = false);
    let report = go_builtin("builtin-lol-scout-on-loading", &host, serde_json::json!({}));
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    assert!(host.asked.lock().unwrap()[0].contains("lol.scout"));
}

/// 旧写法转出来的程序（kotone-core）真的说出和以前一样的话；“让她做”的引号、反斜杠、换行原样交给她。
#[test]
fn converted_old_actions_say_the_same_words() {
    use kotone_core::automation::library::{validate_args, AutomationDraft};
    let convert = |value: serde_json::Value| {
        let draft: AutomationDraft = serde_json::from_value(value).unwrap();
        let args = validate_args(&draft.params, &serde_json::Map::new()).unwrap();
        (draft.action.source().to_string(), args)
    };
    let (source, args) = convert(serde_json::json!({
        "name": "提醒", "params": [
            {"name": "text", "type": "text", "default": "打小龙"},
            {"name": "n", "type": "number", "default": 3},
            {"name": "half", "type": "number", "default": 1.5},
            {"name": "loud", "type": "boolean", "default": false}],
        "action": {"kind": "speak", "text": "该{text}了，{n} 个，{half} 秒，{loud}", "delivery": ["danmaku"]}
    }));
    let host = Fake::new();
    let report = run_with_args(
        &source,
        &args,
        host.clone(),
        Limits::default(),
        Cancel::default(),
    );
    assert_eq!(report.outcome, Outcome::Finished, "{source}: {report:?}");
    assert_eq!(
        *host.toasted.lock().unwrap(),
        vec!["该打小龙了，3 个，1.5 秒，否".to_string()]
    );
    let said = "看看\"对面\"谁是突破口\\不贴标签\n说'一句'";
    let (source, args) = convert(serde_json::json!({
        "name": "突破口", "action": {"kind": "prompt", "text": said}
    }));
    let host = Fake::new();
    let report = run_with_args(
        &source,
        &args,
        host.clone(),
        Limits::default(),
        Cancel::default(),
    );
    assert_eq!(report.outcome, Outcome::Finished, "{source}: {report:?}");
    assert_eq!(*host.asked.lock().unwrap(), vec![said.to_string()]);
}

// ---------- ADR 046：程序里调工具 ----------

fn answering(
    reply: impl Fn(&str, &serde_json::Value) -> Result<ToolReply, String> + Send + Sync + 'static,
) -> Arc<Fake> {
    Fake::with(|f| f.tool_reply = Some(Arc::new(reply)))
}

fn tool_calls(host: &Fake) -> Vec<(String, serde_json::Value)> {
    host.tool_calls
        .lock()
        .unwrap()
        .iter()
        .map(|(_, _, name, args)| (name.clone(), args.clone()))
        .collect()
}

/// 参数转成 JSON 对象交给宿主（整数是整数、列表是数组、不写是 `{}`）；结果是只读的表，没有内容也是真值。
#[test]
fn tool_hands_its_args_to_the_host_and_its_result_back_as_a_read_only_table() {
    use serde_json::json;
    let host = answering(|name, _| {
        Ok(ToolReply::Value(match name {
            "lol.match" => {
                json!({"status": "ok", "alive": {"ally": 1}, "players": [{"champion": "Ahri", "dead": true}]})
            }
            "empty" => serde_json::Value::Null,
            _ => json!("一段文字"),
        }))
    });
    let report = run_with_args(
        r#"
        local r, err = tool("lol.match")
        log(r.players[1].champion .. " " .. tostring(r.players[1].dead) .. " " .. r.alive.ally .. " " .. tostring(err))
        local e = tool("empty")
        if e then log("空的也是真值") end
        log(tool("music", { action = "play", query = args.song, n = 3, half = 1.5, list = { 1, "b" }, opts = { loud = true }, none = {} }))
        local ok = pcall(function() r.status = "改" end)
        if not ok then log("只读") end
        "#,
        &serde_json::from_value(json!({"song": "晴天"})).unwrap(),
        host.clone(),
        Limits::default(),
        Cancel::default(),
    );
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    assert_eq!(
        report.log,
        ["Ahri true 1 nil", "空的也是真值", "一段文字", "只读"]
    );
    let calls = tool_calls(&host);
    assert_eq!(calls[0], ("lol.match".to_string(), json!({})));
    assert_eq!(
        calls[2].1,
        json!({"action": "play", "query": "晴天", "n": 3, "half": 1.5, "list": [1, "b"], "opts": {"loud": true}, "none": {}})
    );
    assert_eq!(calls[2].1["n"].as_i64(), Some(3), "整数交成整数");
}

/// 工具自己说没成 → `nil, 原因`，程序接着跑；调不了（宿主报错）→ 程序出错结束；结果太大、太深当没成。
#[test]
fn tool_failures_come_back_as_nil_and_a_reason_and_refusals_end_the_program() {
    use serde_json::json;
    let deep = |n: usize| (0..n).fold(json!(1), |v, _| json!({ "a": v }));
    let host = answering(move |name, _| match name {
        "slow" => Ok(ToolReply::Failed("超时（30 秒）".into())),
        "blank" => Ok(ToolReply::Failed("  ".into())),
        "big" => Ok(ToolReply::Value(
            json!({"text": "字".repeat(MAX_TOOL_RESULT_BYTES / 3 + 1)}),
        )),
        "deep8" => Ok(ToolReply::Value(deep(MAX_TOOL_RESULT_DEPTH))),
        "deep9" => Ok(ToolReply::Value(deep(MAX_TOOL_RESULT_DEPTH + 1))),
        _ => Err("music 要先在「自动化」页允许".into()),
    });
    let report = go(
        r#"
        for _, name in ipairs({ "slow", "blank", "big", "deep8", "deep9" }) do
          local r, err = tool(name)
          if r then log(name .. " 成了") else log(name .. "：" .. err) end
        end
        "#,
        &host,
    );
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    assert_eq!(report.log[0], "slow：超时（30 秒）");
    assert_eq!(report.log[1], "blank：没成");
    assert!(
        report.log[2].starts_with("big：big 的结果太大"),
        "{:?}",
        report.log
    );
    assert_eq!(report.log[3], "deep8 成了");
    assert!(report.log[4].contains("嵌套太深"), "{:?}", report.log);
    let report = go("tool('music') log('不该到这里')", &host);
    assert!(
        matches!(&report.outcome, Outcome::Error(m) if m.contains("要先在「自动化」页允许")),
        "{report:?}"
    );
    assert!(report.log.is_empty());
    // 宿主没接工具（默认实现）：调不了，程序出错结束
    let report = run(
        "tool('music')",
        Arc::new(Bare),
        Limits::default(),
        Cancel::default(),
    );
    assert!(
        matches!(&report.outcome, Outcome::Error(m) if m.contains("这里调不了工具")),
        "{report:?}"
    );
}

/// 只实现必须实现的方法的宿主（`tool` 用默认实现）。
struct Bare;

impl ScriptHost for Bare {
    fn now_ms(&self) -> u64 {
        0
    }
    fn sleep(&self, _: u64, cancel: &Cancel) -> bool {
        !cancel.is_cancelled()
    }
    fn game_focused(&self) -> bool {
        true
    }
    fn key(&self, _: KeyAction, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn type_text(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn chat(&self, _: &str, _: Option<&str>) -> Result<(), String> {
        Ok(())
    }
    fn brightness(&self, _: Region) -> Result<f64, String> {
        Ok(0.0)
    }
    fn color_at(&self, _: f64, _: f64) -> Result<[u8; 3], String> {
        Ok([0, 0, 0])
    }
    fn pixels(&self, _: Region) -> Result<Pixels, String> {
        Err("没有画面".into())
    }
    fn ocr(&self, _: Region) -> Result<Vec<TextLine>, String> {
        Ok(vec![])
    }
    fn image(&self, _: &str) -> Result<Pixels, String> {
        Err("没有图片".into())
    }
    fn mouse(&self, _: MouseAction) -> Result<(), String> {
        Ok(())
    }
    fn say(&self, _: &str, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn toast(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn push(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn ask(&self, _: &str) -> Result<bool, String> {
        Ok(false)
    }
}

/// 名字和参数先在引擎里查，不合法的不交给宿主，程序出错结束。
#[test]
fn tool_names_and_args_are_checked_before_the_host_sees_them() {
    for source in [
        "tool(1)",
        "tool()",
        "tool('')",
        "tool('a b')",
        "tool('x..y')",
        "tool('.x')",
        "tool('x.')",
        "tool('1x')",
        "tool('中文')",
        &format!("tool('{}')", "a".repeat(MAX_TOOL_NAME_CHARS + 1)),
        "tool('music', 'play')",
        "tool('music', { 1, 2 })",
        "tool('music', { f = function() end })",
        "tool('music', { [true] = 1 })",
        "tool('music', { 1, a = 2 })",
        "tool('music', { list = { [1] = 1, [3] = 3 } })",
        "tool('music', { x = 0/0 })",
        "tool('music', { q = string.rep('a', 5000) })",
        "local t = {} t.self = t tool('music', t)",
    ] {
        let host = Fake::new();
        let report = go(source, &host);
        assert!(
            matches!(&report.outcome, Outcome::Error(m) if m.contains("tool()")),
            "{source}: {report:?}"
        );
        assert!(tool_calls(&host).is_empty(), "{source}");
    }
    for name in [
        "music",
        "lol.match",
        "My-Plugin.Get_Thing",
        &"a".repeat(MAX_TOOL_NAME_CHARS),
    ] {
        let host = Fake::new();
        let report = go(&format!("tool('{name}')"), &host);
        assert_eq!(report.outcome, Outcome::Finished, "{name}: {report:?}");
        assert_eq!(tool_calls(&host)[0].0, name);
    }
}

/// 两次之间至少隔 1 秒（从上一次回来算，太快自动等，不报错）。
#[test]
fn tool_calls_are_spaced_a_second_apart() {
    let host = Fake::with(|f| f.tool_ms = 300);
    let report = go("for i = 1, 3 do tool('lol.match') end", &host);
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    let calls = host.tool_calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 3);
    for pair in calls.windows(2) {
        assert!(pair[1].0 - pair[0].1 >= MIN_TOOL_INTERVAL_MS, "{calls:?}");
    }
    // 中间隔得够久的不用等
    let host = Fake::new();
    go("tool('lol.match') wait(1500) tool('lol.match')", &host);
    let calls = host.tool_calls.lock().unwrap().clone();
    assert_eq!(calls[1].0 - calls[0].1, 1500);
}

/// 停下时正在等的那次记成已取消；宿主停下后还交回了结果也一样，不再往下跑。
#[test]
fn stopping_while_waiting_for_a_tool_is_a_cancel() {
    let cancel = Cancel::default();
    let host = Fake::with(|f| f.interrupt = Some(cancel.clone()));
    let report = run(
        "tool('music') log('不该到这里')",
        host,
        Limits::default(),
        cancel,
    );
    assert_eq!(report.outcome, Outcome::Cancelled, "{report:?}");
    assert!(report.log.is_empty());
    let cancel = Cancel::default();
    let stop = cancel.clone();
    let host = answering(move |_, _| {
        stop.cancel();
        Ok(ToolReply::Failed("没成".into()))
    });
    let report = run(
        "tool('music') log('不该到这里')",
        host,
        Limits::default(),
        cancel,
    );
    assert_eq!(report.outcome, Outcome::Cancelled, "{report:?}");
}

/// 工具名的格式 core（保存时扫描）和引擎（运行时）是同一条规矩：存得下的，运行时也认。
#[test]
fn the_core_scanner_and_the_engine_agree_on_tool_names() {
    use kotone_core::automation::library::{is_tool_name as core_name, tools_called};
    let long = "a".repeat(MAX_TOOL_NAME_CHARS);
    let longer = "a".repeat(MAX_TOOL_NAME_CHARS + 1);
    for name in [
        "music",
        "lol.match",
        "My-Plugin.Get_Thing",
        "a.b.c",
        &long,
        &longer,
        "",
        "1x",
        "x.",
        ".x",
        "x..y",
        "a b",
        "中文",
        "a/b",
        "a:b",
    ] {
        assert_eq!(is_tool_name(name), core_name(name), "{name}");
        let saved = tools_called(&format!("tool(\"{name}\")"));
        assert_eq!(saved.is_ok(), is_tool_name(name), "{name}: {saved:?}");
    }
}

/// ADR 046 的验收程序“队友都倒了放首歌”：放成了不出声；没登录弹一句；没成记日志。不用游戏窗口、不会叫她。
#[test]
fn the_acceptance_program_plays_a_song_or_says_why_not() {
    use kotone_core::automation::library::{tools_called, ProgramUses};
    use serde_json::json;
    let source = "local r = tool(\"music\", { action = \"play\", query = args.song })\nif not r then\n  log(\"没放成\")\nelseif r.status == \"need_login\" then\n  toast(\"网易云音乐还没登录，没放成\")\nend";
    assert_eq!(tools_called(source).unwrap(), ["music"]);
    let uses = ProgramUses::of(source);
    assert!(!uses.window && !uses.asks);
    let args: serde_json::Map<String, serde_json::Value> =
        serde_json::from_value(json!({"song": "Legends Never Die"})).unwrap();
    for (reply, toasted, logged) in [
        (ToolReply::Value(json!({"status": "ok"})), 0, 0),
        (ToolReply::Value(json!({"status": "need_login"})), 1, 0),
        (ToolReply::Failed("没成".into()), 0, 1),
    ] {
        let unbound = Fake::with(|f| {
            f.window = false;
            f.tool_reply = Some(Arc::new(move |_, _| Ok(reply.clone())));
        });
        let report = run_with_args(
            source,
            &args,
            unbound.clone(),
            Limits::default(),
            Cancel::default(),
        );
        assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
        assert_eq!(unbound.toasted.lock().unwrap().len(), toasted);
        assert_eq!(report.log.len(), logged);
        assert_eq!(
            tool_calls(&unbound),
            [(
                "music".to_string(),
                json!({"action": "play", "query": "Legends Never Die"})
            )]
        );
    }
}
