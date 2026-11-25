//! 一次物理按下怎么算（ADR 040 第 4 步删主键位以前补的两处）：
//! 1. 按住组合键时先松修饰键：具名热键和主键位一样——按住的照样等这个键松开才算松开，
//!    中间的重复按下吞掉、不再触发，也不会换成同一个键的另一个热键；
//! 2. 具名热键也有修饰键对不上的诊断（`NamedKeyMissed`），和主键位的 `MainKeyMissed` 对应。

use super::*;

fn down(m: &mut HookMatcher, vk: u32) -> MatchOutcome {
    m.on_key(vk, KeyAction::Down)
}

fn up(m: &mut HookMatcher, vk: u32) -> MatchOutcome {
    m.on_key(vk, KeyAction::Up)
}

const F9: u32 = VK_F1 + 8;
const KEY_Q: u32 = b'Q' as u32;

/// 主键位的事件换成具名热键的说法，好拿两条路逐步比
fn as_named(event: Option<HookEvent>, id: &str) -> Option<HookEvent> {
    event.map(|event| match event {
        HookEvent::HoldPressed => HookEvent::NamedHold {
            id: id.into(),
            pressed: true,
        },
        HookEvent::HoldReleased => HookEvent::NamedHold {
            id: id.into(),
            pressed: false,
        },
        HookEvent::Toggle => HookEvent::NamedHotkey { id: id.into() },
        other => other,
    })
}

#[test]
fn releasing_the_modifier_first_ends_a_named_hold_exactly_like_the_main_slot() {
    let id = HK_HINT_VOICE_INPUT; // 随便一个 id，只看边沿
    for combo in ["Ctrl+F9", "Alt+Mouse4", "Ctrl+Shift+Space"] {
        let spec = parse_hotkey(combo).unwrap();
        let mut main = HookMatcher::new(spec, HotkeyMode::Hold);
        // 具名那条：主键位关着（第 4 步删掉以后就是这样）
        let mut named = HookMatcher::new(parse_hotkey("F8").unwrap(), HotkeyMode::Hold);
        named.set_enabled(false);
        named.set_named_spec_mode(id, Some(spec), HotkeyMode::Hold);

        let mods: Vec<u32> = [
            (spec.ctrl, VK_LCONTROL),
            (spec.alt, VK_LMENU),
            (spec.shift, VK_LSHIFT),
        ]
        .into_iter()
        .filter_map(|(on, vk)| on.then_some(vk))
        .collect();
        // 按下修饰键 → 按下主键 → 先松修饰键 → 主键还在连发 → 松开主键 → 再来一个游离的松开
        let mut steps: Vec<(u32, KeyAction)> =
            mods.iter().map(|&vk| (vk, KeyAction::Down)).collect();
        steps.push((spec.vk, KeyAction::Down));
        steps.extend(mods.iter().map(|&vk| (vk, KeyAction::Up)));
        steps.push((spec.vk, KeyAction::Down));
        steps.push((spec.vk, KeyAction::Down));
        steps.push((spec.vk, KeyAction::Up));
        steps.push((spec.vk, KeyAction::Up));

        let mut named_events = Vec::new();
        for (vk, action) in steps {
            let a = main.on_key(vk, action);
            let b = named.on_key(vk, action);
            assert_eq!(
                a.swallow, b.swallow,
                "{combo}: {vk:#x} {action:?} 吞不吞要和主键位一样"
            );
            assert_eq!(
                as_named(a.event, id),
                b.event,
                "{combo}: {vk:#x} {action:?}"
            );
            named_events.extend(b.event);
        }
        assert_eq!(
            named_events,
            vec![
                HookEvent::NamedHold {
                    id: id.into(),
                    pressed: true
                },
                HookEvent::NamedHold {
                    id: id.into(),
                    pressed: false
                },
            ],
            "{combo}: 按下一次、松开一次，松开在主键松开时"
        );
    }
}

#[test]
fn a_held_key_stays_with_the_hotkey_it_pressed_after_the_modifier_is_released() {
    // 同一个键的两个具名热键：F9（先声明）和 Ctrl+F9（按住）。
    // Ctrl+F9 按住以后先松 Ctrl，F9 的连发不能变成 F9 那个热键。
    let mut m = HookMatcher::new(parse_hotkey("F8").unwrap(), HotkeyMode::Hold);
    m.set_named_spec(HK_CHANNEL_CYCLE, parse_hotkey("F9"));
    m.set_named_spec_mode(HK_ASSISTANT, parse_hotkey("Ctrl+F9"), HotkeyMode::Hold);
    down(&mut m, VK_LCONTROL);
    assert_eq!(
        down(&mut m, F9).event,
        Some(HookEvent::NamedHold {
            id: HK_ASSISTANT.into(),
            pressed: true
        })
    );
    up(&mut m, VK_LCONTROL);
    let repeat = down(&mut m, F9);
    assert!(repeat.swallow, "按住的键连发照样吞");
    assert_eq!(repeat.event, None, "不能换成「F9」那个热键");
    let released = up(&mut m, F9);
    assert!(released.swallow);
    assert_eq!(
        released.event,
        Some(HookEvent::NamedHold {
            id: HK_ASSISTANT.into(),
            pressed: false
        })
    );
    // 松开以后重新按 F9：这回才是 F9 那个热键
    assert_eq!(
        down(&mut m, F9).event,
        Some(HookEvent::NamedHotkey {
            id: HK_CHANNEL_CYCLE.into()
        })
    );
    up(&mut m, F9);

    // 主键位是 Ctrl+F9（按住），具名热键是 F9：主键按住时先松 Ctrl，连发也不能触发具名的 F9
    let mut m = HookMatcher::new(parse_hotkey("Ctrl+F9").unwrap(), HotkeyMode::Hold);
    m.set_named_spec(HK_CHANNEL_CYCLE, parse_hotkey("F9"));
    down(&mut m, VK_LCONTROL);
    assert_eq!(down(&mut m, F9).event, Some(HookEvent::HoldPressed));
    up(&mut m, VK_LCONTROL);
    let repeat = down(&mut m, F9);
    assert!(repeat.swallow);
    assert_eq!(repeat.event, None, "主键按着，连发不归具名热键");
    assert_eq!(up(&mut m, F9).event, Some(HookEvent::HoldReleased));
    assert_eq!(
        down(&mut m, F9).event,
        Some(HookEvent::NamedHotkey {
            id: HK_CHANNEL_CYCLE.into()
        })
    );
}

#[test]
fn a_missed_press_stays_missed_while_held() {
    // 具名热键 F9：按着 Ctrl 按 F9 没命中（放行 + 诊断）；按住时松开 Ctrl，连发不能补触发
    let mut m = HookMatcher::new(parse_hotkey("F8").unwrap(), HotkeyMode::Hold);
    m.set_named_spec(HK_CHANNEL_CYCLE, parse_hotkey("F9"));
    down(&mut m, VK_LCONTROL);
    let missed = down(&mut m, F9);
    assert!(!missed.swallow);
    assert_eq!(
        missed.event,
        Some(HookEvent::NamedKeyMissed {
            id: HK_CHANNEL_CYCLE.into(),
            ctrl: true,
            alt: false,
            shift: false
        })
    );
    up(&mut m, VK_LCONTROL);
    assert_eq!(
        down(&mut m, F9),
        PASS,
        "没命中的那次按下：连发照样放行，不补触发"
    );
    assert_eq!(up(&mut m, F9), PASS);
    // 松开以后正常按：命中
    let hit = down(&mut m, F9);
    assert!(hit.swallow);
    assert_eq!(
        hit.event,
        Some(HookEvent::NamedHotkey {
            id: HK_CHANNEL_CYCLE.into()
        })
    );
}

#[test]
fn named_key_missed_is_reported_once_and_only_with_a_modifier_held() {
    let mut m = HookMatcher::new(parse_hotkey("F8").unwrap(), HotkeyMode::Hold);
    m.set_enabled(false); // 只有具名热键（第 4 步删主键位以后的样子）
    m.set_named_spec_mode(HK_ASSISTANT, parse_hotkey("Alt+Q"), HotkeyMode::Hold);

    // 不带修饰键按 Q：平常打字 / 放技能，放行、不记
    assert_eq!(down(&mut m, KEY_Q), PASS);
    assert_eq!(down(&mut m, KEY_Q), PASS);
    assert_eq!(up(&mut m, KEY_Q), PASS);

    // Shift+Q：对不上 Alt+Q，放行，报一次；连发不再报
    down(&mut m, VK_LSHIFT);
    let missed = down(&mut m, KEY_Q);
    assert!(!missed.swallow, "诊断不吞键");
    assert_eq!(
        missed.event,
        Some(HookEvent::NamedKeyMissed {
            id: HK_ASSISTANT.into(),
            ctrl: false,
            alt: false,
            shift: true
        })
    );
    assert_eq!(down(&mut m, KEY_Q), PASS);
    assert_eq!(up(&mut m, KEY_Q), PASS);
    up(&mut m, VK_LSHIFT);

    // 同一个键另有热键命中了：不算没命中
    m.set_named_spec(HK_CHANNEL_CYCLE, parse_hotkey("Shift+Q"));
    down(&mut m, VK_LSHIFT);
    assert_eq!(
        down(&mut m, KEY_Q).event,
        Some(HookEvent::NamedHotkey {
            id: HK_CHANNEL_CYCLE.into()
        })
    );
    up(&mut m, KEY_Q);
    up(&mut m, VK_LSHIFT);

    // 被动提示键（识别没开）对不上：不报
    let mut m = HookMatcher::new(parse_hotkey("F8").unwrap(), HotkeyMode::Hold);
    m.set_enabled(false);
    m.set_passive_spec(HK_HINT_VOICE_INPUT, parse_hotkey("CapsLock"));
    down(&mut m, VK_LCONTROL);
    assert_eq!(down(&mut m, VK_CAPITAL), PASS);
    up(&mut m, VK_CAPITAL);
    up(&mut m, VK_LCONTROL);

    // 主键位也用这个键：只报主键位自己的 MainKeyMissed，不重复报
    let mut m = HookMatcher::new(parse_hotkey("F9").unwrap(), HotkeyMode::Hold);
    m.set_named_spec(HK_CHANNEL_CYCLE, parse_hotkey("Alt+F9"));
    down(&mut m, VK_LCONTROL);
    assert_eq!(
        down(&mut m, F9).event,
        Some(HookEvent::MainKeyMissed {
            ctrl: true,
            alt: false,
            shift: false
        })
    );
    up(&mut m, F9);
    up(&mut m, VK_LCONTROL);

    // 换键 / 清空以后不留“按着没命中”的旧状态
    let mut m = HookMatcher::new(parse_hotkey("F8").unwrap(), HotkeyMode::Hold);
    m.set_named_spec(HK_CHANNEL_CYCLE, parse_hotkey("F9"));
    down(&mut m, VK_LCONTROL);
    assert!(matches!(
        down(&mut m, F9).event,
        Some(HookEvent::NamedKeyMissed { .. })
    ));
    up(&mut m, VK_LCONTROL);
    m.clear_named_specs();
    m.set_named_spec(HK_CHANNEL_CYCLE, parse_hotkey("F9"));
    assert_eq!(
        down(&mut m, F9).event,
        Some(HookEvent::NamedHotkey {
            id: HK_CHANNEL_CYCLE.into()
        }),
        "清空重配以后，上一次按下没命中的记号不能挡住这次"
    );
}
