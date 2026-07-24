//! 热键规格解析与按键匹配状态机（纯逻辑，与 Win32 解耦，可单测）。
//!
//! LL 钩子后端（hotkey_ll.rs）把 Win32 按键事件翻译为 `on_key` 调用，
//! 本模块决定：是否命中热键、产生什么事件（HookEvent）、是否吞键（swallow）。
//!
//! 设计要点：
//! - 修饰键（Ctrl/Alt/Shift）的实时按下态由状态机自己跟踪（LL 钩子事件流驱动），
//!   命中判定要求修饰键状态与配置**严格相等**（配 F8 时 Ctrl+F8 不命中，不劫持组合键）；
//! - 按住不放产生的重复 down 事件被过滤（自己记录按下态，LL 钩子无 KF_REPEAT）；
//! - 吞键规则：完整命中才吞（down 与对应 up 都吞），未命中的一律放行；
//! - Esc 取消：会话激活（session_active）时 Esc down → Cancel 并吞键，
//!   不再依赖 RegisterHotKey 临时注册。

use crate::hotkey::HotkeyMode;

// ---------- VK 码（Win32 常量值；定义为 u32 常量以便跨平台编译与测试） ----------

pub const VK_BACK: u32 = 0x08;
pub const VK_TAB: u32 = 0x09;
pub const VK_RETURN: u32 = 0x0D;
pub const VK_SHIFT: u32 = 0x10;
pub const VK_CONTROL: u32 = 0x11;
pub const VK_MENU: u32 = 0x12; // Alt
pub const VK_PAUSE: u32 = 0x13;
pub const VK_CAPITAL: u32 = 0x14;
pub const VK_ESCAPE: u32 = 0x1B;
pub const VK_SPACE: u32 = 0x20;
pub const VK_PRIOR: u32 = 0x21; // PageUp
pub const VK_NEXT: u32 = 0x22; // PageDown
pub const VK_END: u32 = 0x23;
pub const VK_HOME: u32 = 0x24;
pub const VK_LEFT: u32 = 0x25;
pub const VK_UP: u32 = 0x26;
pub const VK_RIGHT: u32 = 0x27;
pub const VK_DOWN: u32 = 0x28;
pub const VK_SNAPSHOT: u32 = 0x2C; // PrintScreen
pub const VK_INSERT: u32 = 0x2D;
pub const VK_DELETE: u32 = 0x2E;
pub const VK_LSHIFT: u32 = 0xA0;
pub const VK_RSHIFT: u32 = 0xA1;
pub const VK_LCONTROL: u32 = 0xA2;
pub const VK_RCONTROL: u32 = 0xA3;
pub const VK_LMENU: u32 = 0xA4;
pub const VK_RMENU: u32 = 0xA5;
pub const VK_F1: u32 = 0x70;
// F1..F24 连续：VK_F1 + (n-1)

/// 热键规格：主键 VK 码 + 要求的修饰键状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotkeySpec {
    pub vk: u32,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// 键名 → VK 码（大小写不敏感）；不识别返回 None
fn vk_from_name(name: &str) -> Option<u32> {
    let n = name.trim();
    if n.chars().count() == 1 {
        let c = n.chars().next()?.to_ascii_uppercase();
        match c {
            // A-Z / 0-9 的 VK 码即 ASCII 码
            'A'..='Z' | '0'..='9' => return Some(c as u32),
            _ => {}
        }
    }
    let lower = n.to_ascii_lowercase();
    // F1..F24
    if let Some(num) = lower.strip_prefix('f') {
        if let Ok(k) = num.parse::<u32>() {
            if (1..=24).contains(&k) {
                return Some(VK_F1 + (k - 1));
            }
        }
    }
    let vk = match lower.as_str() {
        "space" => VK_SPACE,
        "tab" => VK_TAB,
        "enter" | "return" => VK_RETURN,
        "esc" | "escape" => VK_ESCAPE,
        "backspace" | "back" => VK_BACK,
        "delete" | "del" => VK_DELETE,
        "insert" | "ins" => VK_INSERT,
        "home" => VK_HOME,
        "end" => VK_END,
        "pageup" | "pgup" => VK_PRIOR,
        "pagedown" | "pgdn" => VK_NEXT,
        "up" => VK_UP,
        "down" => VK_DOWN,
        "left" => VK_LEFT,
        "right" => VK_RIGHT,
        "printscreen" | "print" => VK_SNAPSHOT,
        "pause" => VK_PAUSE,
        "capslock" | "caps" => VK_CAPITAL,
        _ => return None,
    };
    Some(vk)
}

/// 修饰键名（不能作为主键）
fn modifier_from_name(name: &str) -> Option<Modifier> {
    match name.trim().to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(Modifier::Ctrl),
        "alt" | "menu" => Some(Modifier::Alt),
        "shift" => Some(Modifier::Shift),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Modifier {
    Ctrl,
    Alt,
    Shift,
}

/// 解析热键配置串："F8"、"Alt+V"、"Ctrl+Shift+F7"、"Escape"。
/// 主键必须是最后一个分量且不能是修饰键；修饰键顺序无关、重复无害。
pub fn parse_hotkey(key: &str) -> Option<HotkeySpec> {
    let parts: Vec<&str> = key
        .split('+')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    let (main, mods) = parts.split_last()?;
    let vk = vk_from_name(main)?;
    if modifier_from_name(main).is_some() {
        return None; // 主键不能是修饰键（"Alt"、"Ctrl" 单独不作热键）
    }
    let mut spec = HotkeySpec {
        vk,
        ctrl: false,
        alt: false,
        shift: false,
    };
    for m in mods {
        match modifier_from_name(m)? {
            Modifier::Ctrl => spec.ctrl = true,
            Modifier::Alt => spec.alt = true,
            Modifier::Shift => spec.shift = true,
        }
    }
    Some(spec)
}

// ---------- 匹配状态机 ----------

/// 按键动作（LL 钩子事件流抽象）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Down,
    Up,
}

/// 命中后产生的事件（发给 orchestrator）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    /// hold 模式：目标键按下
    HoldPressed,
    /// hold 模式：目标键松开
    HoldReleased,
    /// toggle 模式：目标键按下（已过滤重复）
    Toggle,
    /// 会话激活期间 Esc 取消
    Cancel,
}

/// 一次按键的判定结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchOutcome {
    /// true = 吞掉该键（不 CallNextHookEx，游戏收不到）
    pub swallow: bool,
    pub event: Option<HookEvent>,
}

const PASS: MatchOutcome = MatchOutcome {
    swallow: false,
    event: None,
};

/// 热键匹配状态机：跟踪修饰键实时状态与主键按下态，过滤重复 down。
pub struct HookMatcher {
    spec: HotkeySpec,
    mode: HotkeyMode,
    enabled: bool,
    // 修饰键实时按下态
    ctrl: bool,
    alt: bool,
    shift: bool,
    // 主键物理按下态（重复 down 过滤）
    main_down: bool,
    /// 本次按下的 down 是否命中被吞（对应 up 也要吞）
    main_matched: bool,
    /// hold 模式已发出 HoldPressed（修饰键中途松开也必须补 HoldReleased）
    hold_fired: bool,
    esc_down: bool,
    esc_matched: bool,
    /// 会话激活（state != idle）：Esc 取消使能
    session_active: bool,
}

impl HookMatcher {
    pub fn new(spec: HotkeySpec, mode: HotkeyMode) -> Self {
        Self {
            spec,
            mode,
            enabled: true,
            ctrl: false,
            alt: false,
            shift: false,
            main_down: false,
            main_matched: false,
            hold_fired: false,
            esc_down: false,
            esc_matched: false,
            session_active: false,
        }
    }

    /// 运行时改键/改模式：重置全部按下态
    pub fn set_config(&mut self, spec: HotkeySpec, mode: HotkeyMode) {
        *self = Self::new(spec, mode);
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.main_down = false;
            self.main_matched = false;
            self.hold_fired = false;
            self.esc_down = false;
            self.esc_matched = false;
        }
    }

    pub fn set_session_active(&mut self, active: bool) {
        self.session_active = active;
    }

    fn mods_match(&self) -> bool {
        self.ctrl == self.spec.ctrl && self.alt == self.spec.alt && self.shift == self.spec.shift
    }

    /// 输入一个按键事件，输出吞键与事件判定（钩子回调里唯一入口）
    pub fn on_key(&mut self, vk: u32, action: KeyAction) -> MatchOutcome {
        if !self.enabled {
            return PASS;
        }
        let down = action == KeyAction::Down;

        // 修饰键：只跟踪状态，永不吞键（吞 Ctrl/Alt/Shift 会破坏游戏操作）
        match vk {
            VK_SHIFT | VK_LSHIFT | VK_RSHIFT => {
                self.shift = down;
                return PASS;
            }
            VK_CONTROL | VK_LCONTROL | VK_RCONTROL => {
                self.ctrl = down;
                return PASS;
            }
            VK_MENU | VK_LMENU | VK_RMENU => {
                self.alt = down;
                return PASS;
            }
            _ => {}
        }

        // 主键
        if vk == self.spec.vk {
            return if down {
                if self.main_down {
                    // 按住不放的重复 down：命中过则继续吞（防游戏收到连发），不产生事件
                    MatchOutcome {
                        swallow: self.main_matched,
                        event: None,
                    }
                } else {
                    self.main_down = true;
                    let matched = self.mods_match();
                    self.main_matched = matched;
                    if matched {
                        let event = match self.mode {
                            HotkeyMode::Hold => {
                                self.hold_fired = true;
                                HookEvent::HoldPressed
                            }
                            HotkeyMode::Toggle => HookEvent::Toggle,
                        };
                        MatchOutcome {
                            swallow: true,
                            event: Some(event),
                        }
                    } else {
                        PASS
                    }
                }
            } else {
                if !self.main_down {
                    return PASS; // 游离 up（如禁用时按下的）
                }
                self.main_down = false;
                let swallow = self.main_matched;
                self.main_matched = false;
                if self.mode == HotkeyMode::Hold && self.hold_fired {
                    // hold 已触发：无论修饰键当前状态如何都必须补释放事件并吞键
                    self.hold_fired = false;
                    MatchOutcome {
                        swallow: true,
                        event: Some(HookEvent::HoldReleased),
                    }
                } else {
                    MatchOutcome {
                        swallow,
                        event: None,
                    }
                }
            };
        }

        // Esc 取消（会话激活时；主键本身是 Esc 时上面的分支已接管）
        if vk == VK_ESCAPE {
            return if down {
                if self.esc_down {
                    MatchOutcome {
                        swallow: self.esc_matched,
                        event: None,
                    }
                } else {
                    self.esc_down = true;
                    let matched = self.session_active;
                    self.esc_matched = matched;
                    if matched {
                        MatchOutcome {
                            swallow: true,
                            event: Some(HookEvent::Cancel),
                        }
                    } else {
                        PASS
                    }
                }
            } else {
                if !self.esc_down {
                    return PASS;
                }
                self.esc_down = false;
                let swallow = self.esc_matched;
                self.esc_matched = false;
                MatchOutcome {
                    swallow,
                    event: None,
                }
            };
        }

        PASS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- 键名解析 ----------

    #[test]
    fn parse_function_key() {
        let s = parse_hotkey("F8").unwrap();
        assert_eq!(
            s,
            HotkeySpec {
                vk: 0x77,
                ctrl: false,
                alt: false,
                shift: false
            }
        );
        assert_eq!(parse_hotkey("f1").unwrap().vk, VK_F1);
        assert_eq!(parse_hotkey("F12").unwrap().vk, VK_F1 + 11);
        assert_eq!(parse_hotkey("F24").unwrap().vk, VK_F1 + 23);
        assert!(parse_hotkey("F25").is_none());
        assert!(parse_hotkey("F0").is_none());
    }

    #[test]
    fn parse_letter_and_digit() {
        assert_eq!(parse_hotkey("V").unwrap().vk, u32::from(b'V'));
        assert_eq!(parse_hotkey("v").unwrap().vk, u32::from(b'V'));
        assert_eq!(parse_hotkey("7").unwrap().vk, u32::from(b'7'));
    }

    #[test]
    fn parse_modifier_combos() {
        let s = parse_hotkey("Alt+V").unwrap();
        assert!(s.alt && !s.ctrl && !s.shift && s.vk == u32::from(b'V'));

        let s = parse_hotkey("ctrl + shift + f7").unwrap();
        assert!(s.ctrl && s.shift && !s.alt && s.vk == VK_F1 + 6);

        // 修饰键顺序无关、重复无害
        let s = parse_hotkey("Shift+Ctrl+F7").unwrap();
        assert!(s.ctrl && s.shift);
        let s = parse_hotkey("Ctrl+Ctrl+F7").unwrap();
        assert!(s.ctrl);
    }

    #[test]
    fn parse_named_keys() {
        assert_eq!(parse_hotkey("Escape").unwrap().vk, VK_ESCAPE);
        assert_eq!(parse_hotkey("esc").unwrap().vk, VK_ESCAPE);
        assert_eq!(parse_hotkey("Space").unwrap().vk, VK_SPACE);
        assert_eq!(parse_hotkey("Enter").unwrap().vk, VK_RETURN);
        assert_eq!(parse_hotkey("Tab").unwrap().vk, VK_TAB);
    }

    #[test]
    fn parse_rejects_invalid() {
        assert!(parse_hotkey("").is_none());
        assert!(parse_hotkey("NotAKey").is_none());
        assert!(parse_hotkey("Alt").is_none(), "修饰键不能单独作主键");
        assert!(parse_hotkey("Ctrl+Shift").is_none());
        assert!(parse_hotkey("Win+F8").is_none(), "不支持的修饰键名");
        assert!(parse_hotkey("Alt+NotAKey").is_none());
    }

    // ---------- 匹配状态机 ----------

    fn matcher(key: &str, mode: HotkeyMode) -> HookMatcher {
        HookMatcher::new(parse_hotkey(key).unwrap(), mode)
    }

    fn down(m: &mut HookMatcher, vk: u32) -> MatchOutcome {
        m.on_key(vk, KeyAction::Down)
    }
    fn up(m: &mut HookMatcher, vk: u32) -> MatchOutcome {
        m.on_key(vk, KeyAction::Up)
    }

    #[test]
    fn toggle_plain_key_fires_once_and_swallows() {
        let mut m = matcher("F8", HotkeyMode::Toggle);
        let r = down(&mut m, 0x77);
        assert_eq!(r.event, Some(HookEvent::Toggle));
        assert!(r.swallow);
        // 按住不放的重复 down：吞但不重复发事件
        let r = down(&mut m, 0x77);
        assert_eq!(r.event, None);
        assert!(r.swallow);
        // up：吞（对应被吞的 down），无事件
        let r = up(&mut m, 0x77);
        assert_eq!(r.event, None);
        assert!(r.swallow);
        // 松开后再按：再次触发
        let r = down(&mut m, 0x77);
        assert_eq!(r.event, Some(HookEvent::Toggle));
    }

    #[test]
    fn modifier_must_match_strictly() {
        let mut m = matcher("F8", HotkeyMode::Toggle);
        // 配置无修饰键：Ctrl+F8 不命中、不吞
        down(&mut m, VK_LCONTROL);
        let r = down(&mut m, 0x77);
        assert_eq!(r, PASS);
        up(&mut m, 0x77);
        up(&mut m, VK_LCONTROL);

        // 配置 Alt+V：先按 Alt 再按 V 才命中
        let mut m = matcher("Alt+V", HotkeyMode::Toggle);
        let r = down(&mut m, u32::from(b'V'));
        assert_eq!(r, PASS, "未按 Alt 时不命中");
        up(&mut m, u32::from(b'V'));
        down(&mut m, VK_LMENU);
        let r = down(&mut m, u32::from(b'V'));
        assert_eq!(r.event, Some(HookEvent::Toggle));
        assert!(r.swallow);
        // Alt 本身永不吞（放行给系统/游戏）
        let r = up(&mut m, VK_LMENU);
        assert_eq!(r, PASS);
    }

    #[test]
    fn hold_mode_press_release_pair() {
        let mut m = matcher("F8", HotkeyMode::Hold);
        assert_eq!(down(&mut m, 0x77).event, Some(HookEvent::HoldPressed));
        // 重复 down 不产生第二个 pressed
        assert_eq!(down(&mut m, 0x77).event, None);
        assert_eq!(up(&mut m, 0x77).event, Some(HookEvent::HoldReleased));
        // 第二轮
        assert_eq!(down(&mut m, 0x77).event, Some(HookEvent::HoldPressed));
        assert_eq!(up(&mut m, 0x77).event, Some(HookEvent::HoldReleased));
    }

    #[test]
    fn hold_survives_modifier_released_mid_hold() {
        // Alt+V hold：按住 Alt → 按 V（开始）→ 松开 Alt → 松开 V（仍必须结束会话）
        let mut m = matcher("Alt+V", HotkeyMode::Hold);
        down(&mut m, VK_LMENU);
        assert_eq!(down(&mut m, u32::from(b'V')).event, Some(HookEvent::HoldPressed));
        up(&mut m, VK_LMENU); // Alt 中途松开
        let r = up(&mut m, u32::from(b'V'));
        assert_eq!(r.event, Some(HookEvent::HoldReleased), "hold 已开始就必须补释放");
        assert!(r.swallow);
    }

    #[test]
    fn esc_cancel_only_when_session_active() {
        let mut m = matcher("F8", HotkeyMode::Toggle);
        // 会话未激活：Esc 放行、无事件
        assert_eq!(down(&mut m, VK_ESCAPE), PASS);
        up(&mut m, VK_ESCAPE);

        m.set_session_active(true);
        let r = down(&mut m, VK_ESCAPE);
        assert_eq!(r.event, Some(HookEvent::Cancel));
        assert!(r.swallow, "会话期间吞 Esc，避免触发游戏内菜单");
        // 重复 down 不重复 Cancel
        assert_eq!(down(&mut m, VK_ESCAPE).event, None);
        // up 同样吞掉
        assert!(up(&mut m, VK_ESCAPE).swallow);

        // 会话结束：Esc 恢复放行
        m.set_session_active(false);
        assert_eq!(down(&mut m, VK_ESCAPE), PASS);
    }

    #[test]
    fn disabled_matcher_passes_everything() {
        let mut m = matcher("F8", HotkeyMode::Toggle);
        m.set_enabled(false);
        assert_eq!(down(&mut m, 0x77), PASS);
        m.set_enabled(true);
        // 重新启用后可正常触发（按下态已重置）
        assert_eq!(down(&mut m, 0x77).event, Some(HookEvent::Toggle));
    }

    #[test]
    fn unmatched_keys_pass_through() {
        let mut m = matcher("F8", HotkeyMode::Toggle);
        assert_eq!(down(&mut m, u32::from(b'A')), PASS);
        assert_eq!(down(&mut m, VK_RETURN), PASS);
        assert_eq!(up(&mut m, VK_SPACE), PASS);
    }
}
