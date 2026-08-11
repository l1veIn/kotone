//! 热键端口与纯逻辑：HotkeySource 端口、键名解析、按键匹配状态机（与 Win32 解耦，可单测）。
//!
//! LL 钩子实现（kotone-platform-windows）把 Win32 按键事件翻译为 `on_key` 调用，
//! 本模块决定：是否命中热键、产生什么事件（HookEvent）、是否吞键（swallow）。
//!
//! 设计要点：
//! - 修饰键（Ctrl/Alt/Shift）的实时按下态由状态机自己跟踪（LL 钩子事件流驱动），
//!   命中判定要求修饰键状态与配置**严格相等**（配 F8 时 Ctrl+F8 不命中，不劫持组合键）；
//! - 按住不放产生的重复 down 事件被过滤（自己记录按下态，LL 钩子无 KF_REPEAT）；
//! - 吞键规则：完整命中才吞（down 与对应 up 都吞），未命中的一律放行；
//! - Esc 取消：会话激活（session_active）时 Esc down → Cancel 并吞键，
//!   不再依赖 RegisterHotKey 临时注册。

/// 热键触发模式（用户在设置中选择，默认 toggle 引导时确认）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HotkeyMode {
    /// 按住说话，松手结束
    Hold,
    /// 按一下开始，再按一下结束
    Toggle,
}

/// 热键事件源端口（core）：WH_KEYBOARD_LL 钩子与 RegisterHotKey 是两种实现。
///
/// 实现负责平台线程与按键捕获；事件经构造时注入的 sink 外发（HookEvent），
/// 本端口只定义注册/注销/会话激活开关。register 失败时调用方负责回退。
pub trait HotkeySource: Send + Sync {
    /// 注册/改键（实现内部幂等：已注册则先注销）
    fn register(&self, key: &str, mode: HotkeyMode) -> Result<(), String>;
    /// 注销（实现可保留后台线程以便快速重注册）
    fn unregister(&self);
    /// 会话激活开关：active=true 期间 Esc 取消使能
    fn set_cancel_active(&self, active: bool);
    /// 频道切换热键（ADR-008；可选）：None 或解析失败 = 关闭频道切换。
    /// 默认无操作（不支持切换键的实现安全忽略）。
    fn set_cycle_key(&self, key: Option<&str>) -> Result<(), String> {
        let _ = key;
        Ok(())
    }

    /// 重发最近一条热键（可选）：None 或解析失败 = 关闭。默认无操作。
    fn set_resend_key(&self, key: Option<&str>) -> Result<(), String> {
        let _ = key;
        Ok(())
    }
}

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

impl HotkeySpec {
    /// 组合键显示名："Ctrl+Alt+V" 式，与 `parse_hotkey` 兼容（可 roundtrip 写回配置）
    pub fn combo_name(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".into());
        }
        if self.alt {
            parts.push("Alt".into());
        }
        if self.shift {
            parts.push("Shift".into());
        }
        parts.push(vk_name(self.vk));
        parts.join("+")
    }
}

/// VK 码 → 键名（`vk_from_name` 的反查；未知码兜底 "VK0x.."，不丢信息）
fn vk_name(vk: u32) -> String {
    // A-Z / 0-9 的 VK 码即 ASCII 码
    if (u32::from(b'0')..=u32::from(b'9')).contains(&vk)
        || (u32::from(b'A')..=u32::from(b'Z')).contains(&vk)
    {
        if let Some(c) = char::from_u32(vk) {
            return c.to_string();
        }
    }
    // F1..F24 连续
    if (VK_F1..VK_F1 + 24).contains(&vk) {
        return format!("F{}", vk - VK_F1 + 1);
    }
    let name = match vk {
        VK_SPACE => "Space",
        VK_TAB => "Tab",
        VK_RETURN => "Enter",
        VK_BACK => "Backspace",
        VK_DELETE => "Delete",
        VK_INSERT => "Insert",
        VK_HOME => "Home",
        VK_END => "End",
        VK_PRIOR => "PageUp",
        VK_NEXT => "PageDown",
        VK_UP => "Up",
        VK_DOWN => "Down",
        VK_LEFT => "Left",
        VK_RIGHT => "Right",
        VK_SNAPSHOT => "PrintScreen",
        VK_PAUSE => "Pause",
        VK_CAPITAL => "CapsLock",
        VK_ESCAPE => "Escape",
        _ => return format!("VK0x{vk:02X}"),
    };
    name.to_string()
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
    /// 频道切换键按下（ADR-008；tap 语义，按住不重复触发）
    CycleChannel,
    /// 重发最近一条热键按下（可选；tap 语义，按住不重复触发）
    ResendLast,
    /// 诊断：主键按下但严格修饰键匹配失败（未吞键、不触发业务）。
    /// 「钩子活着但主键被放行」只能靠它区分于「钩子被 Windows 静默摘除」——
    /// 0.1.6 第四步排障：webview 聚焦时 CapsLock 落进 DOM 无钩子上报。
    MainKeyMissed { ctrl: bool, alt: bool, shift: bool },
}

/// 两个热键组合是否冲突（解析后 spec 相等；解析失败不视为冲突，由注册路径报错）
pub fn combos_conflict(a: &str, b: &str) -> bool {
    match (parse_hotkey(a), parse_hotkey(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// 一次按键的判定结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchOutcome {
    /// true = 吞掉该键（不 CallNextHookEx，游戏收不到）
    pub swallow: bool,
    pub event: Option<HookEvent>,
    /// 捕获模式（热键录入）下命中的组合键；非捕获模式恒为 None
    pub captured: Option<HotkeySpec>,
}

const PASS: MatchOutcome = MatchOutcome {
    swallow: false,
    event: None,
    captured: None,
};

/// 热键匹配状态机：跟踪修饰键实时状态与主键按下态，过滤重复 down。
pub struct HookMatcher {
    spec: HotkeySpec,
    mode: HotkeyMode,
    enabled: bool,
    /// 频道切换键（ADR-008；可选）：与主键同构的严格修饰键匹配，tap 语义。
    /// 与主键同 vk 不同修饰键时互不干扰（如主键 CapsLock / 切换 Shift+CapsLock）。
    cycle_spec: Option<HotkeySpec>,
    /// 重发最近一条热键（可选）：与频道切换键同构，tap 语义。
    resend_spec: Option<HotkeySpec>,
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
    // 切换键按下态（重复 down 过滤 + 成对吞键；命中才置位，未命中落回主键分支）
    cycle_down: bool,
    /// 重发键按下态（与 cycle_down 同构）
    resend_down: bool,
    esc_down: bool,
    esc_matched: bool,
    /// 会话激活（state != idle）：Esc 取消使能
    session_active: bool,
    /// 捕获模式（热键录入）：吞掉主键、不产生 HookEvent，非修饰键 down 报告组合键
    capture_active: bool,
    /// 捕获到的主键：捕获结束可能早于物理 keyup，仍需吞掉对应重复 down / up，
    /// 避免 Tab、Space、Enter 等按键落进 WebView 改变焦点或触发控件。
    capture_swallow_vk: Option<u32>,
}

impl HookMatcher {
    pub fn new(spec: HotkeySpec, mode: HotkeyMode) -> Self {
        Self {
            spec,
            mode,
            enabled: true,
            cycle_spec: None,
            resend_spec: None,
            ctrl: false,
            alt: false,
            shift: false,
            main_down: false,
            main_matched: false,
            hold_fired: false,
            cycle_down: false,
            resend_down: false,
            esc_down: false,
            esc_matched: false,
            session_active: false,
            capture_active: false,
            capture_swallow_vk: None,
        }
    }

    /// 运行时改键/改模式：重置全部按下态（保留频道切换键与重发键配置——
    /// 两者由独立设置项驱动，主键改键不应顺手清掉它们）
    pub fn set_config(&mut self, spec: HotkeySpec, mode: HotkeyMode) {
        let cycle_spec = self.cycle_spec;
        let resend_spec = self.resend_spec;
        *self = Self::new(spec, mode);
        self.cycle_spec = cycle_spec;
        self.resend_spec = resend_spec;
    }

    /// 配置/清除频道切换键（None = 关闭频道切换；ADR-008）
    pub fn set_cycle_spec(&mut self, spec: Option<HotkeySpec>) {
        self.cycle_spec = spec;
        self.cycle_down = false;
    }

    /// 配置/清除重发最近一条热键（None = 关闭）
    pub fn set_resend_spec(&mut self, spec: Option<HotkeySpec>) {
        self.resend_spec = spec;
        self.resend_down = false;
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.main_down = false;
            self.main_matched = false;
            self.hold_fired = false;
            self.cycle_down = false;
            self.resend_down = false;
            self.esc_down = false;
            self.esc_matched = false;
        }
    }

    pub fn set_session_active(&mut self, active: bool) {
        self.session_active = active;
    }

    /// 捕获模式开关（热键录入）。捕获期间吞掉被录入的主键、不产生 HookEvent：
    /// 非修饰键 down 报告 `captured` 组合键；Esc down 不捕获（留给调用层当取消信号）。
    /// 捕获优先于 enabled：未注册热键时也能用（CLI/首次设置录入）。
    pub fn set_capture_active(&mut self, active: bool) {
        self.capture_active = active;
    }

    /// 修饰键状态物理同步（LL 钩子回调每个事件前调用，值为 GetAsyncKeyState）。
    /// 修饰键我们从不吞键，物理状态永远准确——以它为准可自愈一切「修饰键
    /// down 收到、up 丢失」（安全桌面 / UAC / 钩链异常）造成的修饰键卡死，
    /// 否则主键会因严格修饰键匹配失败被永久放行（0.1.6 第四步排障发现）。
    /// 只同步修饰键；主键/切换键按下态仍由事件流驱动（主键命中后被吞，
    /// 物理状态无法区分「按住未命中」与「已松开」，不能盲信）。
    pub fn sync_modifiers(&mut self, ctrl: bool, alt: bool, shift: bool) {
        self.ctrl = ctrl;
        self.alt = alt;
        self.shift = shift;
    }

    fn mods_match(&self) -> bool {
        self.ctrl == self.spec.ctrl && self.alt == self.spec.alt && self.shift == self.spec.shift
    }

    /// 输入一个按键事件，输出吞键与事件判定（钩子回调里唯一入口）
    pub fn on_key(&mut self, vk: u32, action: KeyAction) -> MatchOutcome {
        let down = action == KeyAction::Down;

        // 捕获结果会在主键 down 时立即回传，调用方可能在物理 keyup 前关闭捕获。
        // 对这一个主键持续吞到 keyup，不能让 Tab/Space 等尾事件落进 WebView。
        if self.capture_swallow_vk == Some(vk) {
            if !down {
                self.capture_swallow_vk = None;
            }
            return MatchOutcome {
                swallow: true,
                event: None,
                captured: None,
            };
        }

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

        // 捕获模式：录入热键组合（设置页「点击录入」/ CLI --capture）。
        // 吞掉被录入的主键、不产生 HookEvent；Esc down 不捕获（调用层据此发取消信号）。
        if self.capture_active {
            if down && vk != VK_ESCAPE {
                self.capture_swallow_vk = Some(vk);
                return MatchOutcome {
                    swallow: true,
                    event: None,
                    captured: Some(HotkeySpec {
                        vk,
                        ctrl: self.ctrl,
                        alt: self.alt,
                        shift: self.shift,
                    }),
                };
            }
            return PASS;
        }

        if !self.enabled {
            return PASS;
        }

        // 频道切换键（ADR-008）：先于主键判定，但只在确实命中时接管事件——
        // 两者 vk 可相同（如主键 CapsLock、切换 Shift+CapsLock），切换键未命中
        // 必须落回主键分支，否则裸主键（无修饰键的录制键）会被短路吞掉。
        if let Some(cs) = self.cycle_spec {
            if vk == cs.vk {
                if down {
                    if self.cycle_down {
                        // 已命中按住的重复 down：吞但不重复触发
                        return MatchOutcome {
                            swallow: true,
                            event: None,
                            captured: None,
                        };
                    }
                    let matched =
                        self.ctrl == cs.ctrl && self.alt == cs.alt && self.shift == cs.shift;
                    if matched {
                        self.cycle_down = true;
                        return MatchOutcome {
                            swallow: true,
                            event: Some(HookEvent::CycleChannel),
                            captured: None,
                        };
                    }
                    // 未命中：落到主键分支（同 vk 不同修饰键场景）
                } else if self.cycle_down {
                    self.cycle_down = false;
                    return MatchOutcome {
                        swallow: true,
                        event: None,
                        captured: None,
                    };
                }
                // 非切换键按下的 up：落到主键分支
            }
        }

        // 重发最近一条热键：与频道切换键同构（可选；严格修饰键匹配，tap 语义）。
        // 两个辅助键 vk 可相同（如切换 Shift+CapsLock、重发 Alt+CapsLock），
        // 未命中时同样落回主键分支。
        if let Some(rs) = self.resend_spec {
            if vk == rs.vk {
                if down {
                    if self.resend_down {
                        // 已命中按住的重复 down：吞但不重复触发
                        return MatchOutcome {
                            swallow: true,
                            event: None,
                            captured: None,
                        };
                    }
                    let matched =
                        self.ctrl == rs.ctrl && self.alt == rs.alt && self.shift == rs.shift;
                    if matched {
                        self.resend_down = true;
                        return MatchOutcome {
                            swallow: true,
                            event: Some(HookEvent::ResendLast),
                            captured: None,
                        };
                    }
                    // 未命中：落到主键分支（同 vk 不同修饰键场景）
                } else if self.resend_down {
                    self.resend_down = false;
                    return MatchOutcome {
                        swallow: true,
                        event: None,
                        captured: None,
                    };
                }
                // 非重发键按下的 up：落到主键分支
            }
        }

        // 主键
        if vk == self.spec.vk {
            return if down {
                if self.main_down {
                    // 按住不放的重复 down：命中过则继续吞（防游戏收到连发），不产生事件
                    MatchOutcome {
                        swallow: self.main_matched,
                        event: None,
                        captured: None,
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
                            captured: None,
                        }
                    } else {
                        // 严格修饰键匹配失败：不吞键、不触发业务，但上报诊断事件。
                        // 钩子存活但主键被放行（webview 收到 DOM keydown）的唯一
                        // 解释就是修饰键状态卡住，日志据此与「钩子被静默摘除」区分
                        MatchOutcome {
                            swallow: false,
                            event: Some(HookEvent::MainKeyMissed {
                                ctrl: self.ctrl,
                                alt: self.alt,
                                shift: self.shift,
                            }),
                            captured: None,
                        }
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
                        captured: None,
                    }
                } else {
                    MatchOutcome {
                        swallow,
                        event: None,
                        captured: None,
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
                        captured: None,
                    }
                } else {
                    self.esc_down = true;
                    let matched = self.session_active;
                    self.esc_matched = matched;
                    if matched {
                        MatchOutcome {
                            swallow: true,
                            event: Some(HookEvent::Cancel),
                            captured: None,
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
                    captured: None,
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
        // 配置无修饰键：Ctrl+F8 不命中、不吞；上报 MainKeyMissed 诊断事件
        down(&mut m, VK_LCONTROL);
        let r = down(&mut m, 0x77);
        assert!(!r.swallow);
        assert_eq!(
            r.event,
            Some(HookEvent::MainKeyMissed {
                ctrl: true,
                alt: false,
                shift: false
            })
        );
        up(&mut m, 0x77);
        up(&mut m, VK_LCONTROL);

        // 配置 Alt+V：先按 Alt 再按 V 才命中；未按 Alt 不命中、不吞 + 诊断事件
        let mut m = matcher("Alt+V", HotkeyMode::Toggle);
        let r = down(&mut m, u32::from(b'V'));
        assert!(!r.swallow, "未按 Alt 时不命中");
        assert_eq!(
            r.event,
            Some(HookEvent::MainKeyMissed {
                ctrl: false,
                alt: false,
                shift: false
            })
        );
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
        assert_eq!(
            down(&mut m, u32::from(b'V')).event,
            Some(HookEvent::HoldPressed)
        );
        up(&mut m, VK_LMENU); // Alt 中途松开
        let r = up(&mut m, u32::from(b'V'));
        assert_eq!(
            r.event,
            Some(HookEvent::HoldReleased),
            "hold 已开始就必须补释放"
        );
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

    // ---------- 捕获模式（热键录入） ----------

    #[test]
    fn combo_name_roundtrip() {
        for s in [
            "F7",
            "F24",
            "Alt+V",
            "Ctrl+Shift+F7",
            "Ctrl+Alt+Shift+A",
            "Space",
            "Up",
            "PrintScreen",
        ] {
            let spec = parse_hotkey(s).unwrap();
            assert_eq!(
                parse_hotkey(&spec.combo_name()).unwrap(),
                spec,
                "roundtrip: {s}"
            );
        }
        // 未知 VK 码兜底 VK0x..（不可 parse 但不丢信息）
        assert_eq!(
            HotkeySpec {
                vk: 0xFF,
                ctrl: false,
                alt: false,
                shift: false
            }
            .combo_name(),
            "VK0xFF"
        );
    }

    #[test]
    fn capture_reports_combo_with_modifiers() {
        let mut m = matcher("F8", HotkeyMode::Toggle);
        m.set_capture_active(true);
        // 修饰键照常跟踪、不吞
        assert_eq!(down(&mut m, VK_LCONTROL), PASS);
        assert_eq!(down(&mut m, VK_LMENU), PASS);
        let r = down(&mut m, u32::from(b'V'));
        assert_eq!(
            r.captured,
            Some(HotkeySpec {
                vk: u32::from(b'V'),
                ctrl: true,
                alt: true,
                shift: false,
            })
        );
        assert_eq!(r.captured.unwrap().combo_name(), "Ctrl+Alt+V");
        // 松开修饰键后再按：组合态更新
        up(&mut m, VK_LCONTROL);
        up(&mut m, VK_LMENU);
        let r = down(&mut m, 0x77);
        assert_eq!(r.captured.unwrap().combo_name(), "F8");
    }

    #[test]
    fn capture_esc_is_not_captured() {
        let mut m = matcher("F8", HotkeyMode::Toggle);
        m.set_capture_active(true);
        // Esc down 不捕获（调用层当取消信号）、不吞键
        let r = down(&mut m, VK_ESCAPE);
        assert_eq!(r, PASS);
        assert_eq!(r.captured, None);
    }

    #[test]
    fn capture_swallows_main_key_and_fires_no_events() {
        let mut m = matcher("F8", HotkeyMode::Toggle);
        m.set_capture_active(true);
        m.set_session_active(true);
        // 即使按下已注册的主键：报告 captured 而非 HookEvent，并吞键
        let r = down(&mut m, 0x77);
        assert!(r.swallow);
        assert_eq!(r.event, None);
        assert!(r.captured.is_some());
        // 捕获层通常会在 down 后立即结束；对应的重复 down / up 仍必须吞掉。
        m.set_capture_active(false);
        assert!(down(&mut m, 0x77).swallow);
        let r = up(&mut m, 0x77);
        assert!(r.swallow);
        assert_eq!(r.event, None);
        // Esc 在捕获期也不发 Cancel（录入优先于会话取消）
        m.set_capture_active(true);
        assert_eq!(down(&mut m, VK_ESCAPE), PASS);
        // 结束捕获后恢复正常匹配
        m.set_capture_active(false);
        assert_eq!(down(&mut m, 0x77).event, Some(HookEvent::Toggle));
    }

    #[test]
    fn capture_works_when_disabled() {
        // 未注册热键（matcher disabled）时也能录入（CLI / 首次设置场景）
        let mut m = matcher("F24", HotkeyMode::Toggle);
        m.set_enabled(false);
        m.set_capture_active(true);
        let r = down(&mut m, u32::from(b'G'));
        assert_eq!(r.captured.unwrap().combo_name(), "G");
    }

    // ---------- 频道切换键（ADR-008） ----------

    #[test]
    fn cycle_key_fires_with_strict_modifiers() {
        let mut m = matcher("CapsLock", HotkeyMode::Toggle);
        m.set_cycle_spec(parse_hotkey("Shift+CapsLock"));
        // 无 Shift：命中主键而非切换键
        let r = down(&mut m, VK_CAPITAL);
        assert_eq!(r.event, Some(HookEvent::Toggle));
        assert!(r.swallow);
        up(&mut m, VK_CAPITAL);
        // Shift+CapsLock：命中切换键，不触发录制
        down(&mut m, VK_LSHIFT);
        let r = down(&mut m, VK_CAPITAL);
        assert_eq!(r.event, Some(HookEvent::CycleChannel));
        assert!(r.swallow);
        // 重复 down 不重复触发
        assert_eq!(down(&mut m, VK_CAPITAL).event, None);
        // up 吞键、无事件
        let r = up(&mut m, VK_CAPITAL);
        assert!(r.swallow);
        assert_eq!(r.event, None);
        up(&mut m, VK_LSHIFT);
        // Shift 本身永不吞（放行给游戏）
    }

    #[test]
    fn cycle_key_rejects_extra_modifiers() {
        let mut m = matcher("F8", HotkeyMode::Toggle);
        m.set_cycle_spec(parse_hotkey("Shift+F9"));
        // Ctrl+Shift+F9：修饰键不严格相等，不命中
        down(&mut m, VK_LCONTROL);
        down(&mut m, VK_LSHIFT);
        let r = down(&mut m, VK_F1 + 8);
        assert_eq!(r, PASS);
        up(&mut m, VK_F1 + 8);
        up(&mut m, VK_LSHIFT);
        up(&mut m, VK_LCONTROL);
    }

    #[test]
    fn cycle_spec_survives_main_rekey_and_clears_on_none() {
        let mut m = matcher("CapsLock", HotkeyMode::Toggle);
        m.set_cycle_spec(parse_hotkey("Shift+CapsLock"));
        // 主键改键不清掉切换键
        m.set_config(parse_hotkey("F8").unwrap(), HotkeyMode::Toggle);
        down(&mut m, VK_LSHIFT);
        assert_eq!(
            down(&mut m, VK_CAPITAL).event,
            Some(HookEvent::CycleChannel)
        );
        up(&mut m, VK_CAPITAL);
        up(&mut m, VK_LSHIFT);
        // 清除后不再触发
        m.set_cycle_spec(None);
        down(&mut m, VK_LSHIFT);
        assert_eq!(down(&mut m, VK_CAPITAL), PASS);
        up(&mut m, VK_CAPITAL);
        up(&mut m, VK_LSHIFT);
    }

    #[test]
    fn cycle_key_disabled_with_matcher() {
        let mut m = matcher("CapsLock", HotkeyMode::Toggle);
        m.set_cycle_spec(parse_hotkey("Shift+CapsLock"));
        m.set_enabled(false);
        down(&mut m, VK_LSHIFT);
        assert_eq!(down(&mut m, VK_CAPITAL), PASS, "运行时停止后切换键也放行");
        up(&mut m, VK_CAPITAL);
        up(&mut m, VK_LSHIFT);
    }

    #[test]
    fn combos_conflict_detects_same_spec() {
        assert!(combos_conflict("Shift+CapsLock", "shift + capslock"));
        assert!(combos_conflict("Ctrl+Shift+F7", "Shift+Ctrl+F7"));
        assert!(!combos_conflict("CapsLock", "Shift+CapsLock"));
        assert!(!combos_conflict("F8", "F9"));
        // 解析失败不视为冲突（注册路径会单独报错）
        assert!(!combos_conflict("NotAKey", "F8"));
    }

    // ---------- 重发最近一条热键 ----------

    #[test]
    fn resend_key_fires_with_strict_modifiers() {
        let mut m = matcher("F8", HotkeyMode::Toggle);
        m.set_resend_spec(parse_hotkey("Alt+F9"));
        // 无 Alt：不命中、不吞（F9 非主键 → 整体放行）
        let r = down(&mut m, VK_F1 + 8);
        assert_eq!(r, PASS);
        up(&mut m, VK_F1 + 8);
        // Alt+F9：命中重发键
        down(&mut m, VK_LMENU);
        let r = down(&mut m, VK_F1 + 8);
        assert_eq!(r.event, Some(HookEvent::ResendLast));
        assert!(r.swallow);
        // 重复 down 不重复触发
        assert_eq!(down(&mut m, VK_F1 + 8).event, None);
        // up 吞键、无事件
        let r = up(&mut m, VK_F1 + 8);
        assert!(r.swallow);
        assert_eq!(r.event, None);
        up(&mut m, VK_LMENU);
        // 松开后再次按：再次触发
        down(&mut m, VK_LMENU);
        assert_eq!(down(&mut m, VK_F1 + 8).event, Some(HookEvent::ResendLast));
        up(&mut m, VK_F1 + 8);
        up(&mut m, VK_LMENU);
    }

    #[test]
    fn resend_key_rejects_extra_modifiers() {
        let mut m = matcher("F8", HotkeyMode::Toggle);
        m.set_resend_spec(parse_hotkey("Alt+F9"));
        // Ctrl+Alt+F9：修饰键不严格相等，不命中
        down(&mut m, VK_LCONTROL);
        down(&mut m, VK_LMENU);
        let r = down(&mut m, VK_F1 + 8);
        assert_eq!(r, PASS);
        up(&mut m, VK_F1 + 8);
        up(&mut m, VK_LMENU);
        up(&mut m, VK_LCONTROL);
    }

    #[test]
    fn resend_and_cycle_same_vk_different_modifiers() {
        // 主键 CapsLock、切换 Shift+CapsLock、重发 Alt+CapsLock：三者互不干扰
        let mut m = matcher("CapsLock", HotkeyMode::Toggle);
        m.set_cycle_spec(parse_hotkey("Shift+CapsLock"));
        m.set_resend_spec(parse_hotkey("Alt+CapsLock"));
        // 裸 CapsLock：主键
        let r = down(&mut m, VK_CAPITAL);
        assert_eq!(r.event, Some(HookEvent::Toggle));
        up(&mut m, VK_CAPITAL);
        // Shift+CapsLock：切换键
        down(&mut m, VK_LSHIFT);
        let r = down(&mut m, VK_CAPITAL);
        assert_eq!(r.event, Some(HookEvent::CycleChannel));
        up(&mut m, VK_CAPITAL);
        up(&mut m, VK_LSHIFT);
        // Alt+CapsLock：重发键
        down(&mut m, VK_LMENU);
        let r = down(&mut m, VK_CAPITAL);
        assert_eq!(r.event, Some(HookEvent::ResendLast));
        up(&mut m, VK_CAPITAL);
        up(&mut m, VK_LMENU);
    }

    #[test]
    fn resend_spec_survives_main_rekey_and_clears_on_none() {
        let mut m = matcher("CapsLock", HotkeyMode::Toggle);
        m.set_resend_spec(parse_hotkey("Alt+F9"));
        // 主键改键不清掉重发键
        m.set_config(parse_hotkey("F8").unwrap(), HotkeyMode::Toggle);
        down(&mut m, VK_LMENU);
        assert_eq!(down(&mut m, VK_F1 + 8).event, Some(HookEvent::ResendLast));
        up(&mut m, VK_F1 + 8);
        up(&mut m, VK_LMENU);
        // 清除后不再触发
        m.set_resend_spec(None);
        down(&mut m, VK_LMENU);
        assert_eq!(down(&mut m, VK_F1 + 8), PASS);
        up(&mut m, VK_F1 + 8);
        up(&mut m, VK_LMENU);
    }

    #[test]
    fn resend_key_disabled_with_matcher() {
        let mut m = matcher("F8", HotkeyMode::Toggle);
        m.set_resend_spec(parse_hotkey("Alt+F9"));
        m.set_enabled(false);
        down(&mut m, VK_LMENU);
        assert_eq!(down(&mut m, VK_F1 + 8), PASS, "运行时停止后重发键也放行");
        up(&mut m, VK_F1 + 8);
        up(&mut m, VK_LMENU);
    }
}
