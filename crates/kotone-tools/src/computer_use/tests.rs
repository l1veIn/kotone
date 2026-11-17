use std::cell::RefCell;

use serde_json::json;

use super::actions::{normalize_keys, parse_action, parse_plan, Action, Button, Target};
use super::annotate::{annotate, label_size, place_labels, resize, text_color};
use super::batch::{execute, validate, ActHost, Halt, StepStatus, StopReason};
use super::observe::{
    elements_from_text, scale_for, zoom_region, CoordError, Observation, TextBox,
};
use crate::image::RgbaImage;

// ---------- 缩放 ----------

#[test]
fn scale_follows_the_anthropic_formula() {
    // 1920×1080：长边限制 1280/1920 = 0.667，像素限制 sqrt(1.15e6/2073600) = 0.745 → 0.667
    let s = scale_for(1920, 1080, 1280, 1_150_000);
    assert!((s.factor - 1280.0 / 1920.0).abs() < 1e-9);
    assert_eq!((s.width, s.height), (1280, 720));
    // 小窗口不放大
    let s = scale_for(800, 600, 1280, 1_150_000);
    assert_eq!((s.width, s.height, s.factor), (800, 600, 1.0));
    // 正方形大图由像素上限决定
    let s = scale_for(1200, 1200, 1280, 1_000_000);
    assert!((s.factor - (1_000_000f64 / 1_440_000.0).sqrt()).abs() < 1e-9);
}

fn obs(elements: Vec<super::observe::Element>) -> Observation {
    Observation {
        id: "o1".into(),
        client_width: 1920,
        client_height: 1080,
        image_width: 1280,
        image_height: 720,
        elements,
    }
}

#[test]
fn screenshot_pixels_map_back_to_window_ratio() {
    let o = obs(Vec::new());
    let (x, y) = o.to_ratio(640.0, 360.0).unwrap();
    assert!((x - 640.5 / 1280.0).abs() < 1e-12 && (y - 360.5 / 720.0).abs() < 1e-12);
    // 换回客户区像素：640.5/1280 × 1920 ≈ 960.75 → 落在原图 960 附近
    assert_eq!((x * 1920.0) as i32, 960);
    assert!(matches!(
        o.to_ratio(1280.0, 10.0),
        Err(CoordError::OutOfImage { .. })
    ));
    assert!(o.to_ratio(-1.0, 10.0).is_err());
}

// ---------- 编号清单 ----------

#[test]
fn text_lines_become_numbered_elements_in_reading_order() {
    let lines = vec![
        TextBox {
            text: "确定".into(),
            ratio: [0.6, 0.5, 0.1, 0.05],
        },
        TextBox {
            text: "  ".into(),
            ratio: [0.1, 0.1, 0.1, 0.05],
        },
        TextBox {
            text: "取消".into(),
            ratio: [0.2, 0.505, 0.1, 0.05],
        },
        TextBox {
            text: "标题".into(),
            ratio: [0.4, 0.1, 0.2, 0.05],
        },
    ];
    let e = elements_from_text(&lines, 1000, 1000);
    let texts: Vec<&str> = e.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(texts, ["标题", "取消", "确定"]);
    assert_eq!(e.iter().map(|e| e.id).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(e[0].bbox, [400, 100, 600, 150]);
    let o = obs(e);
    assert_eq!(o.element(2).unwrap().text, "取消");
    assert!(matches!(o.element(9), Err(CoordError::UnknownElement(9))));
}

#[test]
fn zoom_region_is_checked_against_the_full_screenshot() {
    assert_eq!(
        zoom_region(1280, 720, [0.0, 0.0, 640.0, 360.0]).unwrap(),
        [0.0, 0.0, 0.5, 0.5]
    );
    assert!(zoom_region(1280, 720, [10.0, 10.0, 5.0, 20.0]).is_err());
    assert!(zoom_region(1280, 720, [0.0, 0.0, 1281.0, 20.0]).is_err());
    assert!(zoom_region(1280, 720, [0.0, 0.0, 4.0, 4.0]).is_err());
}

// ---------- 标签 ----------

#[test]
fn label_text_color_follows_omniparser_luminance_rule() {
    assert_eq!(text_color([255, 225, 25]), [0, 0, 0]); // 黄底黑字
    assert_eq!(text_color([0x43, 0x63, 0xD8]), [255, 255, 255]); // 蓝底白字
}

#[test]
fn labels_try_above_then_left_then_right_then_above_right() {
    let (lw, lh) = label_size(1);
    // 框上方有空间：放在上方左对齐
    let pos = place_labels(&[(1, [50, 50, 100, 80])], 200, 200);
    assert_eq!(pos, vec![(50, 50 - i64::from(lh))]);
    // 贴着顶边：上方放不下，改到框外左侧
    let pos = place_labels(&[(1, [50, 0, 100, 30])], 200, 200);
    assert_eq!(pos, vec![(50 - i64::from(lw), 0)]);
    // 贴着左上角：左侧也放不下，改到框外右侧
    let pos = place_labels(&[(1, [0, 0, 40, 30])], 200, 200);
    assert_eq!(pos, vec![(40, 0)]);
}

#[test]
fn annotate_draws_box_and_label_without_changing_size() {
    let image = RgbaImage::new(64, 64, vec![255; 64 * 64 * 4]).unwrap();
    let out = annotate(&image, &[(7, [20, 30, 50, 50])]);
    assert_eq!((out.width(), out.height()), (64, 64));
    // 框的左上角是调色板颜色（编号 7 → 第 7 个颜色）
    let [r, g, b, _] = out.pixel(20, 30).unwrap();
    assert_eq!([r, g, b], super::annotate::color_for(7));
    // 原图没变
    assert_eq!(image.pixel(20, 30).unwrap(), [255, 255, 255, 255]);
}

#[test]
fn resize_and_jpeg_round_trip_sizes() {
    let image = RgbaImage::new(40, 20, vec![128; 40 * 20 * 4]).unwrap();
    let small = resize(&image, 20, 10);
    assert_eq!((small.width(), small.height()), (20, 10));
    let jpeg = super::annotate::encode_jpeg(&small, 80).unwrap();
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
}

// ---------- 动作 ----------

#[test]
fn actions_copy_anthropic_names_and_parameters() {
    assert_eq!(
        parse_action(&json!({"action": "left_click", "coordinate": [10, 20], "text": "shift"}))
            .unwrap(),
        Action::Click {
            button: Button::Left,
            count: 1,
            target: Some(Target::Coordinate(10.0, 20.0)),
            modifiers: Some("Shift".into()),
        }
    );
    assert_eq!(
        parse_action(&json!({"action": "double_click", "element": 3}))
            .unwrap()
            .name(),
        "double_click"
    );
    assert_eq!(
        parse_action(&json!({"action": "key", "text": "ctrl+s"})).unwrap(),
        Action::Key {
            keys: "Ctrl+s".into(),
            repeat: 1
        }
    );
    assert_eq!(
        parse_action(&json!({"action": "hold_key", "text": "w", "duration": 1.5})).unwrap(),
        Action::HoldKey {
            keys: "w".into(),
            duration_ms: 1500
        }
    );
    assert_eq!(
        parse_action(&json!({"action": "mouse_move_by", "dx": -30, "dy": 5})).unwrap(),
        Action::MouseMoveBy { dx: -30, dy: 5 }
    );
    for bad in [
        json!({"action": "left_click", "coordinate": [1, 2], "element": 1}),
        json!({"action": "hold_key", "text": "w", "duration": 11}),
        json!({"action": "scroll", "scroll_direction": "sideways"}),
        json!({"action": "type", "text": ""}),
        json!({"action": "screenshot"}),
        json!({"action": "teleport"}),
        json!({"action": "mouse_move_by", "dx": 9999, "dy": 0}),
    ] {
        assert!(parse_action(&bad).is_err(), "{bad}");
    }
}

#[test]
fn xdotool_key_names_are_normalized() {
    assert_eq!(normalize_keys("Return"), "Enter");
    assert_eq!(
        normalize_keys("ctrl+shift+Page_Down"),
        "Ctrl+Shift+PageDown"
    );
    assert_eq!(normalize_keys("BackSpace"), "Backspace");
    assert_eq!(normalize_keys("super"), "Win");
    assert_eq!(normalize_keys("F5"), "F5");
}

#[test]
fn a_plan_has_one_to_twenty_steps() {
    assert!(parse_plan(&json!([])).is_err());
    let many: Vec<_> = (0..21)
        .map(|_| json!({"action": "wait", "duration": 0}))
        .collect();
    assert!(parse_plan(&json!(many)).is_err());
    let err =
        parse_plan(&json!([{"action": "wait", "duration": 0}, {"action": "nope"}])).unwrap_err();
    assert!(err.starts_with("第 2 步"), "{err}");
    let long: Vec<_> = (0..3)
        .map(|_| json!({"action": "hold_key", "text": "w", "duration": 8}))
        .collect();
    assert!(parse_plan(&json!(long)).unwrap_err().contains("合计"));
}

// ---------- 批量执行 ----------

#[derive(Default)]
struct FakeHost {
    log: RefCell<Vec<String>>,
    /// 第几次 checkpoint 时报告停下（从 0 开始）。
    stop_at_checkpoint: Option<(usize, StopReason)>,
    checkpoints: RefCell<usize>,
    /// 这个键按下时失败。
    failing_key: Option<String>,
    /// sleep 时报告停下（模拟按住键时玩家接手）。
    stop_in_sleep: Option<StopReason>,
    covered: bool,
}

impl ActHost for FakeHost {
    fn key_known(&self, keys: &str) -> bool {
        !keys.contains("Bogus")
    }
    fn checkpoint(&self) -> Result<(), Halt> {
        let n = *self.checkpoints.borrow();
        *self.checkpoints.borrow_mut() += 1;
        match self.stop_at_checkpoint {
            Some((at, reason)) if at == n => Err(Halt::Stop(reason)),
            _ => Ok(()),
        }
    }
    fn move_to(&self, x: f64, y: f64) -> Result<(), Halt> {
        if self.covered {
            return Err(Halt::Stop(StopReason::PointCovered));
        }
        self.log.borrow_mut().push(format!("move {x:.3} {y:.3}"));
        Ok(())
    }
    fn button(&self, button: Button, down: bool) -> Result<(), Halt> {
        self.log.borrow_mut().push(format!(
            "button {button:?} {}",
            if down { "down" } else { "up" }
        ));
        Ok(())
    }
    fn click(&self, button: Button, count: u8) -> Result<(), Halt> {
        self.log
            .borrow_mut()
            .push(format!("click {button:?} x{count}"));
        Ok(())
    }
    fn wheel(&self, direction: super::actions::ScrollDirection, amount: u32) -> Result<(), Halt> {
        self.log
            .borrow_mut()
            .push(format!("wheel {direction:?} {amount}"));
        Ok(())
    }
    fn type_text(&self, text: &str) -> Result<(), Halt> {
        self.log
            .borrow_mut()
            .push(format!("type {}", text.chars().count()));
        Ok(())
    }
    fn key_press(&self, keys: &str) -> Result<(), Halt> {
        if self.failing_key.as_deref() == Some(keys) {
            return Err(Halt::Fail(format!("{keys} 发送失败")));
        }
        self.log.borrow_mut().push(format!("press {keys}"));
        Ok(())
    }
    fn key_down(&self, keys: &str) -> Result<(), Halt> {
        self.log.borrow_mut().push(format!("down {keys}"));
        Ok(())
    }
    fn key_up(&self, keys: &str) -> Result<(), Halt> {
        self.log.borrow_mut().push(format!("up {keys}"));
        Ok(())
    }
    fn move_by(&self, dx: i32, dy: i32) -> Result<(), Halt> {
        self.log.borrow_mut().push(format!("move_by {dx} {dy}"));
        Ok(())
    }
    fn sleep(&self, ms: u64) -> Result<(), Halt> {
        if let Some(reason) = self.stop_in_sleep {
            if ms > 200 {
                return Err(Halt::Stop(reason));
            }
        }
        self.log.borrow_mut().push(format!("sleep {ms}"));
        Ok(())
    }
}

fn plan(v: serde_json::Value) -> Vec<Action> {
    parse_plan(&v).unwrap()
}

fn element_obs() -> Observation {
    obs(vec![super::observe::Element {
        id: 1,
        text: "背包".into(),
        bbox: [100, 100, 140, 120],
    }])
}

#[test]
fn whole_plan_is_validated_before_anything_is_sent() {
    let host = FakeHost::default();
    let o = element_obs();
    let p = plan(json!([
        {"action": "key", "text": "i"},
        {"action": "left_click", "element": 5}
    ]));
    let err = validate(&p, &o, &host).unwrap_err();
    assert!(err.contains("第 2 步") && err.contains("编号 5"), "{err}");
    let p = plan(json!([{"action": "key", "text": "ctrl+Bogus"}]));
    assert!(validate(&p, &o, &host).unwrap_err().contains("Bogus"));
    let p = plan(json!([{"action": "left_click", "coordinate": [2000, 10]}]));
    assert!(validate(&p, &o, &host).unwrap_err().contains("超出截图"));
    assert!(host.log.borrow().is_empty());
}

#[test]
fn click_moves_then_settles_then_clicks_with_modifiers() {
    let host = FakeHost::default();
    let o = element_obs();
    let p = plan(json!([{"action": "left_click", "element": 1, "text": "shift"}]));
    let report = execute(&p, &o, &host, &|_, _| {});
    assert_eq!(report.stop, None);
    assert_eq!(report.steps[0].status, StepStatus::Sent);
    let (x, y) = o.resolve(Target::Element(1)).unwrap();
    assert_eq!(
        *host.log.borrow(),
        [
            format!("move {x:.3} {y:.3}"),
            "sleep 100".into(),
            "down Shift".into(),
            "click Left x1".into(),
            "up Shift".into(),
        ]
    );
}

#[test]
fn first_failure_stops_and_later_steps_are_not_executed() {
    let host = FakeHost {
        failing_key: Some("e".into()),
        ..Default::default()
    };
    let p = plan(json!([
        {"action": "key", "text": "i"},
        {"action": "key", "text": "e"},
        {"action": "key", "text": "q"}
    ]));
    let report = execute(&p, &element_obs(), &host, &|_, _| {});
    let statuses: Vec<_> = report.steps.iter().map(|s| s.status).collect();
    assert_eq!(
        statuses,
        [
            StepStatus::Sent,
            StepStatus::Failed,
            StepStatus::NotExecuted
        ]
    );
    assert_eq!(
        report.steps[2].message.as_deref(),
        Some("Not executed: an earlier action failed.")
    );
    assert_eq!(report.stop, Some(StopReason::ActionFailed));
    assert!(!host.log.borrow().iter().any(|l| l == "press q"));
}

#[test]
fn player_input_between_steps_keeps_finished_steps() {
    let host = FakeHost {
        stop_at_checkpoint: Some((1, StopReason::PlayerInput)),
        ..Default::default()
    };
    let p = plan(json!([
        {"action": "key", "text": "i"},
        {"action": "key", "text": "e"}
    ]));
    let report = execute(&p, &element_obs(), &host, &|_, _| {});
    assert_eq!(report.stop, Some(StopReason::PlayerInput));
    assert_eq!(report.steps[0].status, StepStatus::Sent);
    assert_eq!(report.steps[1].status, StepStatus::NotExecuted);
    let json = super::report_json(&report);
    assert_eq!(json["stopReason"], "player_input");
    assert_eq!(json["sent"], 1);
}

#[test]
fn held_keys_are_released_when_interrupted_or_left_down() {
    // 按住 w 时玩家接手：w 一定松开
    let host = FakeHost {
        stop_in_sleep: Some(StopReason::PlayerInput),
        ..Default::default()
    };
    let p = plan(json!([{"action": "hold_key", "text": "w", "duration": 3}]));
    let report = execute(&p, &element_obs(), &host, &|_, _| {});
    assert_eq!(report.steps[0].status, StepStatus::Stopped);
    assert_eq!(*host.log.borrow(), ["down w", "up w"]);

    // key_down 没有配对的 key_up、左键按下没松开：结束时都松开
    let host = FakeHost::default();
    let p = plan(json!([
        {"action": "key_down", "text": "shift"},
        {"action": "left_mouse_down"}
    ]));
    execute(&p, &element_obs(), &host, &|_, _| {});
    assert_eq!(
        *host.log.borrow(),
        [
            "down Shift",
            "button Left down",
            "up Shift",
            "button Left up"
        ]
    );
}

#[test]
fn covered_point_is_not_clicked() {
    let host = FakeHost {
        covered: true,
        ..Default::default()
    };
    let p = plan(json!([{"action": "left_click", "coordinate": [10, 10]}]));
    let report = execute(&p, &element_obs(), &host, &|_, _| {});
    assert_eq!(report.stop, Some(StopReason::PointCovered));
    assert_eq!(report.steps[0].status, StepStatus::NotExecuted);
    assert!(host.log.borrow().is_empty());
}

#[test]
fn typing_goes_in_groups_of_fifty() {
    let host = FakeHost::default();
    let text: String = "字".repeat(120);
    let p = plan(json!([{"action": "type", "text": text}]));
    execute(&p, &element_obs(), &host, &|_, _| {});
    assert_eq!(
        *host.log.borrow(),
        ["type 50", "sleep 12", "type 50", "sleep 12", "type 20"]
    );
}

#[test]
fn drag_presses_moves_in_steps_and_releases() {
    let host = FakeHost::default();
    let p = plan(
        json!([{"action": "left_click_drag", "start_coordinate": [0, 0], "coordinate": [100, 0]}]),
    );
    execute(&p, &element_obs(), &host, &|_, _| {});
    let log = host.log.borrow();
    assert_eq!(log[2], "button Left down");
    assert_eq!(log.last().unwrap(), "button Left up");
    assert_eq!(log.iter().filter(|l| l.starts_with("move ")).count(), 11);
}
