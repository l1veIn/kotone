//! 发到游戏（ADR 041）：“把这段话发到当前游戏的聊天”只有这一个入口。
//!
//! 用它的有：语音输入（玩家自己说的话）、她和外部 Agent（`game_chat.send`）、自动化的 `chat()`（旧工具
//! `companion.type` 和旧陪玩流程的“帮忙打字”以前也用，ADR 053 退掉了）。入口里统一做（D4–D6）：
//!
//! 1. 整理文字：一行一条，丢掉空行，最多 [`MAX_LINES`] 行、每行 [`MAX_LINE_CHARS`] 字；
//! 2. 挑发法（D3，第 2 期）：前台（语音输入：开始收音的那个窗口）是某个游戏插件登记的窗口（例如英雄联盟客户端）
//!    → 交给插件（[`provider`]），不按键，下面 3–7 都不做；插件关着时替玩家发言的不发、说开哪个开关，玩家自己的
//!    语音输入照旧往下走。插件接手以后失败了也不退回按键；
//! 3. 拿宿主那一把输入锁（[`crate::input_lock`]），排队最多 5 秒、不超过调用方的截止时间；
//! 4. 等修饰键松开（每行最多 1 秒）；按住 Shift 时开聊天框的回车会变成 Shift+Enter（全体频道）；
//! 5. 有 `restore` 就拿到锁以后再切回那个窗口（语音输入：回到开始收音时的窗口，D5）；
//! 6. 按**前台检测到的游戏**挑游戏配置（[`pick_game`]），再挑频道（[`pick_channel`]）；
//! 7. 逐行发：每行之前看取消、截止时间、前台还是不是同一个窗口；一行一个 ADR 032 的挡键窗口；行间 65 毫秒；
//! 8. 结果说清楚（[`ChatOutcome`]）：走了哪条路、哪个频道、发了几行、没发完的原因。
//!
//! **不重试、不补发**（ADR 032、原则 4）：没发出去的就说没发出去。调用方拿到的结果总是真的——
//! 截止时间之前一定返回，开始一行之前估计来不及就不开始。

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::inject::{CancelToken, FocusBackend, InjectError, Injector, TargetWindow};
pub use crate::inject::{ForegroundInfo, SendRoute};
use crate::input_lock::{InputBusy, InputLock, InputUser, LockError, INPUT_LOCK_WAIT};
use crate::process_log::EventData;
use crate::profile::{self, ChannelStrategy, GameProfile};
use crate::settings::Settings;

pub mod provider;
pub use provider::{ChatProvider, ChatSend, ChatSent, ChatStop, SendToGameSpec, StopCode};

/// 一次最多几行：英雄联盟“战绩一条一行”，一局 10 个人。
pub const MAX_LINES: usize = 10;
/// 每行最多几个字（和以前的单行上限一样）。
pub const MAX_LINE_CHARS: usize = 200;
/// 工具参数 `text` 的上限（10 行 × 200 字）。
pub const MAX_TEXT_CHARS: usize = MAX_LINES * MAX_LINE_CHARS;
/// 行间隔：LeagueAkari 的默认 `sendInterval = 65`（`in-game-send/state.ts`）。
pub const LINE_INTERVAL: Duration = Duration::from_millis(65);
/// 修饰键最多等多久（每行）：语音输入原来的值，ADR 032 C4“最多 1 秒，等不到就不发”。
pub const MODIFIER_WAIT: Duration = Duration::from_millis(1000);
const MODIFIER_POLL: Duration = Duration::from_millis(10);
/// 调用方没给截止时间时（她、语音输入）：排队最多 5 秒，加上发几行的时间。
/// 比语音输入的发送看门狗（10 秒）短，正常情况下看门狗不会先到。
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(8);

/// 估一行要多久：每个键按住 50 毫秒加调度、挡键窗口的尾巴 30 毫秒、留一点余量。
const KEY_PRESS_MS: u64 = 60;
const LINE_TAIL_MS: u64 = 30;
const LINE_MARGIN_MS: u64 = 100;
/// 剪贴板粘贴后恢复原内容前的等待（`inject.rs` 的 `CLIPBOARD_RESTORE_DELAY_MS`）。
const CLIPBOARD_RESTORE_MS: u64 = 100;

/// 频道 id：游戏配置里的语义 id（跨游戏通用，`ProfileChannel.id`）。
pub const CHANNEL_TEAM: &str = "team";
pub const CHANNEL_ALL: &str = "all";

/// 谁在发：写进锁的占用者；决定前台不对时拦不拦（[`pick_game`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaker {
    /// 语音输入：玩家自己说的话，打进他开始收音的那个窗口。
    Player,
    /// 她替玩家发言。
    Companion,
    /// 外部 Agent（工具服务、MCP）。
    Agent,
    /// 自动化（`chat()`）。
    Automation,
}

/// 频道怎么选。`for_game` = 这个频道是在哪个游戏配置里选的（语音输入记的是 (游戏, 频道)）：
/// 和挑中的游戏不是同一个时退回默认频道。工具传 None：频道 id 跨游戏通用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelPick {
    pub id: String,
    pub for_game: Option<String>,
}

/// 拿到锁以后先切回这个窗口、等 `settle` 再发。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestoreFocus {
    pub window: TargetWindow,
    pub settle: Duration,
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    /// 要发的话；多行时一行发一条。
    pub text: String,
    /// None = 游戏配置的默认频道（内置配置都是“队伍”）。
    pub channel: Option<ChannelPick>,
    /// false = 只打进聊天框、不按发送键（只能一行）。
    pub send: bool,
    pub speaker: Speaker,
    pub restore: Option<RestoreFocus>,
    /// 这个时间之前一定返回（工具服务：它自己的期限减一点余量）；None = [`DEFAULT_DEADLINE`]。
    pub deadline: Option<Instant>,
    pub cancel: CancelToken,
    /// 只发到要发去的那个窗口（语音输入推送卡片上的“重发”：玩家在聊天里点的，这时前台多半是 Kotone 自己，ADR 043 D10）：
    /// 切不回 `restore` 那个窗口（关了、切不过去）就不发；前台是 Kotone 自己、桌面、没有前台也不发——不退回“发到
    /// 现在的前台”，免得打进 Kotone 自己的输入框。默认 false：语音输入照旧（窗口没了就发到现在的前台）。
    pub strict_target: bool,
}

impl ChatRequest {
    pub fn new(text: impl Into<String>, speaker: Speaker) -> Self {
        Self {
            text: text.into(),
            channel: None,
            send: true,
            speaker,
            restore: None,
            deadline: None,
            cancel: CancelToken::default(),
            strict_target: false,
        }
    }
}

/// 发了（至少一行）以后的结果。
#[derive(Debug, Clone)]
pub struct ChatOutcome {
    pub route: SendRoute,
    /// 提供发法的插件（走插件的发法时，例如 `lol`）；本体的通用发法是 None
    pub plugin: Option<String>,
    /// 用的游戏配置
    pub game: String,
    pub game_name: String,
    /// 实际用的频道
    pub channel: String,
    pub channel_name: String,
    /// 语音输入选的频道属于别的游戏，退回了默认频道。
    pub channel_fell_back: bool,
    /// 整理后的行
    pub lines: Vec<String>,
    /// 发完的行数（`send = false` 时是打进聊天框的行数）
    pub sent_lines: usize,
    /// 全部行都发了
    pub sent: bool,
    /// 发了一部分就停了：为什么
    pub stopped: Option<ChatFailure>,
    /// 排队、等修饰键一共等了多久
    pub waited_ms: u64,
}

impl ChatOutcome {
    /// 给她 / 外部 Agent 的结果（短，原则 10）。
    pub fn to_json(&self) -> Value {
        let mut value = json!({
            "sent": self.sent,
            "route": self.route.as_str(),
            "game": self.game,
            "gameName": self.game_name,
            "channel": self.channel,
            "channelName": self.channel_name,
            "lines": self.lines.len(),
            "sentLines": self.sent_lines,
            "waitedMs": self.waited_ms,
        });
        if let Some(plugin) = &self.plugin {
            value["plugin"] = json!(plugin);
        }
        if self.channel_fell_back {
            value["channelFellBack"] = json!(true);
        }
        if let Some(stopped) = &self.stopped {
            value["stopped"] = json!({
                "code": stopped.code(),
                "message": format!(
                    "后面 {} 行没发：{stopped}",
                    self.lines.len() - self.sent_lines
                ),
            });
        }
        value
    }
}

/// 没发出去（或者没发完）的原因。`Display` 是给玩家 / 她看的话，[`Self::code`] 给记录和过程日志。
#[derive(Debug, Clone)]
pub enum ChatFailure {
    EmptyText,
    TooManyLines {
        lines: usize,
        max: usize,
    },
    LineTooLong {
        line: usize,
        max: usize,
    },
    MultiLineWithoutSend,
    NoForeground,
    OwnWindow,
    Desktop,
    /// 替玩家发言时前台不是游戏
    NotAGame {
        process: String,
        game: String,
    },
    /// 游戏配置里没有要的频道（要“所有人”但没有）
    NoSuchChannel {
        channel: String,
        game: String,
    },
    /// Ctrl / Alt / Shift / Win 1 秒内没松开
    ModifiersHeld,
    /// 发到一半前台换了窗口
    ForegroundChanged,
    /// 排队排到截止时间还没轮到
    Busy(InputBusy),
    /// 来不及在截止时间前发完下一行
    TimeUp,
    Cancelled,
    Inject(InjectError),
    /// 插件的发法没发（或没发完）：插件给的原因（[`StopCode`]）。不退回按键
    Provider(ChatStop),
    /// 前台是插件登记的窗口，但插件或它要的开关关着；替玩家发言的不往这个窗口里按键
    ProviderOff {
        process: String,
        /// “英雄联盟插件的「在客户端里发聊天」关着”
        why: String,
    },
    /// 这里经插件的发法发（客户端接口），没法只打进聊天框不发（`send = false`）
    NoDraftHere,
    /// 只发到要发去的那个窗口（[`ChatRequest::strict_target`]）：那个窗口关了，或者切不过去
    WindowGone,
}

impl ChatFailure {
    pub fn code(&self) -> &'static str {
        match self {
            Self::EmptyText => "emptyText",
            Self::TooManyLines { .. } => "tooManyLines",
            Self::LineTooLong { .. } => "lineTooLong",
            Self::MultiLineWithoutSend => "multiLineWithoutSend",
            Self::NoForeground => "noForeground",
            Self::OwnWindow => "ownWindow",
            Self::Desktop => "desktop",
            Self::NotAGame { .. } => "notAGame",
            Self::NoSuchChannel { .. } => "noSuchChannel",
            Self::ModifiersHeld => "modifiersHeld",
            Self::ForegroundChanged => "foregroundChanged",
            Self::Busy(_) => "busy",
            Self::TimeUp => "timeUp",
            Self::Cancelled => "cancelled",
            Self::Inject(e) if e.input_blocked => "inputBlocked",
            Self::Inject(e) if e.needs_elevation => "needsElevation",
            Self::Inject(_) => "injectFailed",
            Self::Provider(stop) => stop.code.as_str(),
            Self::ProviderOff { .. } => "providerOff",
            Self::NoDraftHere => "noDraftHere",
            Self::WindowGone => "windowGone",
        }
    }

    /// 参数不对（调用方改参数就能好），其余是执行时的情况。
    pub fn is_invalid_arguments(&self) -> bool {
        matches!(
            self,
            Self::EmptyText
                | Self::TooManyLines { .. }
                | Self::LineTooLong { .. }
                | Self::MultiLineWithoutSend
        )
    }
}

impl std::fmt::Display for ChatFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const SWITCH: &str = "请先切到游戏，再让我替你发言";
        match self {
            Self::EmptyText => write!(f, "要发的话是空的"),
            Self::TooManyLines { lines, max } => {
                write!(f, "一次最多发 {max} 行，这次有 {lines} 行")
            }
            Self::LineTooLong { line, max } => write!(f, "第 {line} 行超过 {max} 字"),
            Self::MultiLineWithoutSend => write!(f, "只打进聊天框、不发送时只能一行"),
            Self::NoForeground => write!(f, "当前没有前台窗口：{SWITCH}"),
            Self::OwnWindow => write!(f, "前台是 Kotone 自己的窗口：{SWITCH}"),
            Self::Desktop => write!(f, "前台是桌面或文件夹，不是游戏：{SWITCH}"),
            Self::NotAGame { process, game } => {
                write!(f, "前台是 {process}，不是「{game}」：{SWITCH}")
            }
            Self::NoSuchChannel { channel, game } => write!(
                f,
                "「{game}」的游戏配置里没有「{channel}」频道，这次没发：可以在游戏列表里给它加一个"
            ),
            Self::ModifiersHeld => write!(
                f,
                "Ctrl、Alt、Shift 或 Win 一直按着（等了 1 秒），这次没发：松开再发"
            ),
            Self::ForegroundChanged => write!(f, "前台换了窗口，没往新窗口里发"),
            Self::Busy(busy) => write!(f, "{busy}"),
            Self::TimeUp => write!(f, "时间不够了（调用的期限快到了），没开始发"),
            Self::Cancelled => write!(f, "发送已取消"),
            Self::Inject(e) => write!(f, "{}", e.message),
            Self::Provider(stop) => write!(f, "{stop}"),
            Self::ProviderOff { process, why } => write!(
                f,
                "前台是 {process}，{why}，这次没替你发：打开它，或者切到游戏里再让我替你发言"
            ),
            Self::NoDraftHere => write!(
                f,
                "这里的聊天经客户端发，没法只打进聊天框不发，这次没发：要直接发，打开“帮忙打字后自动发送”（唤醒 → 快捷指令）"
            ),
            Self::WindowGone => write!(f, "要发去的那个窗口已经关了（或者切不过去），这次没发"),
        }
    }
}

/// 宿主交给编排器（语音输入）和桌面端各处的端口。阻塞执行，异步调用方放进 `spawn_blocking`。
pub trait SendToGame: Send + Sync {
    fn send(&self, request: ChatRequest) -> Result<ChatOutcome, ChatFailure>;
}

/// 时钟（测试不真睡）。
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
    fn sleep(&self, duration: Duration);
}

pub struct RealClock;

impl Clock for RealClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

type ProfileSource = Arc<dyn Fn() -> Vec<GameProfile> + Send + Sync>;
type Note = Arc<dyn Fn(&str, EventData) + Send + Sync>;

/// 本体的实现：通用发法（开聊天框 → 逐字 Unicode → 发送，D2），用游戏配置里的聊天键、频道和时序。
pub struct GameChat {
    injector: Arc<dyn Injector>,
    focus: Arc<dyn FocusBackend>,
    lock: Arc<InputLock>,
    settings: Arc<RwLock<Settings>>,
    profiles: ProfileSource,
    clock: Arc<dyn Clock>,
    note: Note,
    /// 游戏插件提供的发法（D3，第 2 期；桌面端从内置插件表建）。
    providers: Vec<Arc<dyn ChatProvider>>,
}

impl GameChat {
    /// `lock` 必须是宿主那一份（和 `game.act`、按键程序共用）。
    pub fn new(
        injector: Arc<dyn Injector>,
        focus: Arc<dyn FocusBackend>,
        lock: Arc<InputLock>,
        settings: Arc<RwLock<Settings>>,
    ) -> Self {
        Self {
            injector,
            focus,
            lock,
            settings,
            profiles: Arc::new(profile::list),
            clock: Arc::new(RealClock),
            note: Arc::new(|activity, data| crate::process_log::record_activity(activity, data)),
            providers: Vec::new(),
        }
    }

    /// 游戏插件提供的发法（桌面端：内置插件清单里的 `[[send_to_game]]`）。
    pub fn with_providers(mut self, providers: Vec<Arc<dyn ChatProvider>>) -> Self {
        self.providers = providers;
        self
    }

    /// 游戏配置从哪来（测试给固定的；生产是 `profile::list`）。
    pub fn with_profiles(
        mut self,
        profiles: impl Fn() -> Vec<GameProfile> + Send + Sync + 'static,
    ) -> Self {
        self.profiles = Arc::new(profiles);
        self
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// 过程日志写到哪（测试不写盘）。
    pub fn with_note(mut self, note: impl Fn(&str, EventData) + Send + Sync + 'static) -> Self {
        self.note = Arc::new(note);
        self
    }

    pub fn lock(&self) -> &Arc<InputLock> {
        &self.lock
    }

    /// 等 Ctrl / Alt / Shift / Win 松开：最多 1 秒；`reserve` = 松开以后还要留给这一行的时间
    /// （到截止时间不够了就 `TimeUp`）。返回等了多久（毫秒）。
    fn wait_modifiers(
        &self,
        cancel: &CancelToken,
        deadline: Instant,
        reserve: Duration,
    ) -> Result<u64, ChatFailure> {
        if cancel.is_cancelled() {
            return Err(ChatFailure::Cancelled);
        }
        if !self.focus.modifier_keys_down() {
            return Ok(0);
        }
        crate::log::log("inject: waiting for Ctrl/Alt/Shift/Win to release");
        let started = self.clock.now();
        loop {
            if cancel.is_cancelled() {
                return Err(ChatFailure::Cancelled);
            }
            let waited = self.clock.now().saturating_duration_since(started);
            if !self.focus.modifier_keys_down() {
                (self.note)(
                    "inject_wait_modifiers",
                    EventData {
                        detail: Some(format!("waitedMs={}", waited.as_millis())),
                        outcome: Some("ok".into()),
                        ..Default::default()
                    },
                );
                return Ok(waited.as_millis() as u64);
            }
            if waited >= MODIFIER_WAIT {
                crate::log::log("inject: modifiers still down after 1000ms; not sending");
                (self.note)(
                    "inject_wait_modifiers",
                    EventData {
                        detail: Some("timeoutMs=1000".into()),
                        outcome: Some("timeout".into()),
                        ..Default::default()
                    },
                );
                return Err(ChatFailure::ModifiersHeld);
            }
            if self.clock.now() + reserve + MODIFIER_POLL > deadline {
                return Err(ChatFailure::TimeUp);
            }
            self.clock.sleep(MODIFIER_POLL);
        }
    }

    /// 当前选中的游戏配置和挑游戏要的配置列表（只在用得上时才读盘）。
    fn games(&self, fg: Option<&ForegroundInfo>) -> (GameProfile, Vec<GameProfile>) {
        let active_id = self.settings.read().unwrap().active_profile_id.clone();
        let need_list = active_id.is_some() || fg.is_some_and(|f| f.process.is_some());
        let profiles = if need_list {
            (self.profiles)()
        } else {
            Vec::new()
        };
        let active = active_id
            .and_then(|id| profiles.iter().find(|p| p.id == id).cloned())
            .unwrap_or_else(GameProfile::builtin_generic);
        (active, profiles)
    }

    /// 这次的话归不归某个插件的发法（D3）：看语音输入开始收音的那个窗口（窗口没了就看现在的前台），否则看
    /// 现在的前台；它的程序在某个插件登记的窗口里 → (那个发法, 程序名)。Kotone 自己的窗口不归谁。
    fn provider_for(&self, request: &ChatRequest) -> Option<(&Arc<dyn ChatProvider>, String)> {
        if self.providers.is_empty() {
            return None;
        }
        let info = request
            .restore
            .and_then(|r| self.focus.window_info(r.window))
            .or_else(|| self.focus.foreground_info())?;
        if info.own_process {
            return None;
        }
        let process = info.process?;
        self.providers
            .iter()
            .find(|p| p.spec().covers(&process))
            .map(|p| (p, process))
    }

    /// 交给插件的发法：不拿锁、不等修饰键、不切窗口（不按键）。插件的结果就是最终结果，不退回按键（D6、ADR 032）。
    fn send_via_provider(
        &self,
        provider: &dyn ChatProvider,
        request: &ChatRequest,
        lines: Vec<String>,
        deadline: Instant,
    ) -> Result<ChatOutcome, ChatFailure> {
        if !request.send {
            return Err(ChatFailure::NoDraftHere);
        }
        if request.cancel.is_cancelled() {
            return Err(ChatFailure::Cancelled);
        }
        let spec = provider.spec();
        let game = (self.profiles)().into_iter().find(|p| p.id == spec.game);
        let (channel, channel_fell_back) = match &game {
            Some(game) => requested_channel(game, request.channel.as_ref()),
            None => (
                request
                    .channel
                    .as_ref()
                    .map_or(CHANNEL_TEAM.to_string(), |c| c.id.clone()),
                false,
            ),
        };
        let result = provider.send(ChatSend {
            game: spec.game.clone(),
            lines: lines.clone(),
            channel,
            deadline,
            cancel: request.cancel.clone(),
        });
        let result = match result {
            // 守住“Ok 至少发了一行”
            Ok(sent) if sent.sent_lines == 0 => Err(sent.stopped.unwrap_or_else(|| {
                ChatStop::new(StopCode::ClientRejected, "客户端说一行都没发出去，这次没发")
            })),
            other => other,
        };
        (self.note)(
            "send_to_game_provider",
            EventData {
                detail: Some(format!(
                    "plugin={} game={} lines={}",
                    provider.plugin(),
                    spec.game,
                    lines.len()
                )),
                outcome: Some(match &result {
                    Ok(sent) if sent.stopped.is_none() => "ok".into(),
                    Ok(_) => "partial".into(),
                    Err(stop) => stop.code.as_str().into(),
                }),
                ..Default::default()
            },
        );
        let sent = result.map_err(ChatFailure::Provider)?;
        let sent_lines = sent.sent_lines.min(lines.len());
        Ok(ChatOutcome {
            route: SendRoute::Client,
            plugin: Some(provider.plugin().to_string()),
            game: spec.game.clone(),
            game_name: game.map_or_else(|| spec.game.clone(), |g| g.display_name),
            channel: sent.channel,
            channel_name: sent.channel_name,
            channel_fell_back,
            sent: sent_lines == lines.len() && sent.stopped.is_none(),
            stopped: sent.stopped.map(ChatFailure::Provider),
            lines,
            sent_lines,
            waited_ms: 0,
        })
    }
}

impl SendToGame for GameChat {
    fn send(&self, request: ChatRequest) -> Result<ChatOutcome, ChatFailure> {
        let started = self.clock.now();
        let lines = split_lines(&request.text, request.send)?;
        let deadline = request.deadline.unwrap_or(started + DEFAULT_DEADLINE);
        let cancel = request.cancel.clone();

        // 挑发法（D3，第 2 期）：前台是插件登记的窗口 → 交给插件，不按键
        if let Some((provider, process)) = self.provider_for(&request) {
            match provider.off() {
                None => {
                    return self.send_via_provider(provider.as_ref(), &request, lines, deadline)
                }
                // 替玩家发言的：不往插件登记的窗口里按键（客户端里按 Enter 可能点到有焦点的按钮）
                Some(why) if request.speaker != Speaker::Player => {
                    return Err(ChatFailure::ProviderOff { process, why })
                }
                // 玩家自己的语音输入：照旧按键（玩家拍板过“前台是什么窗就往哪发”）
                Some(_) => {}
            }
        }

        // 排队：最多 5 秒，不超过截止时间。锁按真实时间等（测试的假时钟只管睡觉）。
        let lock_wait = deadline
            .saturating_duration_since(self.clock.now())
            .min(INPUT_LOCK_WAIT);
        let queued = Instant::now();
        let _guard = self
            .lock
            .acquire(
                InputUser::Chat(request.speaker),
                queued + lock_wait,
                &|| cancel.is_cancelled(),
            )
            .map_err(|e| match e {
                LockError::Busy(busy) => ChatFailure::Busy(busy),
                LockError::Cancelled => ChatFailure::Cancelled,
            })?;
        let mut waited_ms = queued.elapsed().as_millis() as u64;

        // 第一行之前先等修饰键（在切回窗口之前：等的时候取消了就不切）
        waited_ms += self.wait_modifiers(&cancel, deadline, Duration::ZERO)?;
        let mut restored = None;
        if let Some(restore) = request.restore {
            let back = self.focus.restore(restore.window);
            if back && !restore.settle.is_zero() {
                self.clock.sleep(restore.settle);
            }
            restored = Some((restore.window, back));
        }
        let fg = self.focus.foreground_info();
        if request.strict_target {
            strict_front(fg.as_ref(), restored)?;
        }
        let (active, profiles) = self.games(fg.as_ref());
        let game = pick_game(fg.as_ref(), &profiles, &active, request.speaker)?;
        let (strategy, channel_fell_back) = pick_channel(&game, request.channel.as_ref())?;
        let mut wire = game.for_channel(&strategy);
        if !request.send {
            wire.send_key.clear();
        }
        let window = fg.map(|f| f.window);

        let mut outcome = ChatOutcome {
            route: SendRoute::Keys,
            plugin: None,
            game: game.id.clone(),
            game_name: game.display_name.clone(),
            channel: strategy.id.clone(),
            channel_name: strategy.display_name.clone(),
            channel_fell_back,
            lines: lines.clone(),
            sent_lines: 0,
            sent: false,
            stopped: None,
            waited_ms: 0,
        };
        for (i, line) in lines.iter().enumerate() {
            let text = format!("{}{line}", strategy.text_prefix);
            let estimate = estimate_line(&wire, &text);
            let result = (|| {
                if cancel.is_cancelled() {
                    return Err(ChatFailure::Cancelled);
                }
                if i > 0 {
                    // 行与行之间挡键窗口是关的：玩家这时按下的 Shift 会到游戏，不等就是 Shift+Enter
                    waited_ms += self.wait_modifiers(&cancel, deadline, estimate)?;
                    if window.is_some() && self.focus.foreground_window() != window {
                        return Err(ChatFailure::ForegroundChanged);
                    }
                }
                if self.clock.now() + estimate > deadline {
                    return Err(ChatFailure::TimeUp);
                }
                self.injector
                    .send_line(&text, &wire, cancel.clone())
                    .map_err(ChatFailure::Inject)
            })();
            match result {
                Ok(route) => {
                    outcome.route = route;
                    outcome.sent_lines += 1;
                }
                Err(failure) if outcome.sent_lines == 0 => return Err(failure),
                Err(failure) => {
                    outcome.stopped = Some(failure);
                    break;
                }
            }
            if i + 1 < lines.len() {
                self.clock.sleep(LINE_INTERVAL);
            }
        }
        outcome.sent = outcome.sent_lines == lines.len();
        outcome.waited_ms = waited_ms;
        Ok(outcome)
    }
}

/// [`ChatRequest::strict_target`] 的检查（拿到锁、切回窗口以后，按键以前）：要切回的那个窗口切回来了、就在前台；
/// 前台不是 Kotone 自己、不是桌面或文件夹。
fn strict_front(
    fg: Option<&ForegroundInfo>,
    restored: Option<(TargetWindow, bool)>,
) -> Result<(), ChatFailure> {
    if let Some((window, back)) = restored {
        if !back || fg.map(|f| f.window) != Some(window) {
            return Err(ChatFailure::WindowGone);
        }
    }
    let fg = fg.ok_or(ChatFailure::NoForeground)?;
    if fg.own_process {
        return Err(ChatFailure::OwnWindow);
    }
    let process = fg.process.as_deref().unwrap_or("");
    if process.is_empty() || process.eq_ignore_ascii_case("explorer.exe") {
        return Err(ChatFailure::Desktop);
    }
    Ok(())
}

/// 一行一条：`\r\n` / `\n` / `\r` 都算换行，每行去首尾空白，丢掉空行（照 LeagueAkari 的
/// `lines.filter((line) => line.trim().length > 0)`）。
pub fn split_lines(text: &str, send: bool) -> Result<Vec<String>, ChatFailure> {
    let lines: Vec<String> = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    if lines.is_empty() {
        return Err(ChatFailure::EmptyText);
    }
    if lines.len() > MAX_LINES {
        return Err(ChatFailure::TooManyLines {
            lines: lines.len(),
            max: MAX_LINES,
        });
    }
    if let Some(i) = lines
        .iter()
        .position(|l| l.chars().count() > MAX_LINE_CHARS)
    {
        return Err(ChatFailure::LineTooLong {
            line: i + 1,
            max: MAX_LINE_CHARS,
        });
    }
    if !send && lines.len() > 1 {
        return Err(ChatFailure::MultiLineWithoutSend);
    }
    Ok(lines)
}

/// 按前台挑游戏配置（ADR 041 D3 / D5）：
///
/// 1. 前台程序对得上某个**具体**游戏配置（`processNames` 不空）→ 用它（选中英雄联盟、前台是 Dota 2 时按 Dota 2 的键发）；
/// 2. 对不上 → 用当前选中的；
/// 3. 替玩家发言的（她、外部 Agent、自动化）再拦一道：前台是 Kotone 自己、桌面 / 资源管理器，或者选中的是具体游戏
///    而前台不是它 → 不发（2026-11-18 她往桌面打字的教训）。选中“通用（任意前台窗口）”时照旧什么前台都行；
/// 4. 玩家自己的语音输入不拦（玩家拍板：前台是什么窗就往哪发，记事本里测英雄联盟的配置也要能发）。
pub fn pick_game(
    fg: Option<&ForegroundInfo>,
    profiles: &[GameProfile],
    active: &GameProfile,
    speaker: Speaker,
) -> Result<GameProfile, ChatFailure> {
    let process = fg.and_then(|f| f.process.as_deref()).unwrap_or("");
    let on_behalf = speaker != Speaker::Player;
    if on_behalf {
        let fg = fg.ok_or(ChatFailure::NoForeground)?;
        if fg.own_process {
            return Err(ChatFailure::OwnWindow);
        }
        if process.is_empty() || process.eq_ignore_ascii_case("explorer.exe") {
            return Err(ChatFailure::Desktop);
        }
    }
    if !process.is_empty() {
        if let Some(game) = profiles
            .iter()
            .find(|p| !p.process_names.is_empty() && profile::matches_process(p, process))
        {
            return Ok(game.clone());
        }
    }
    if on_behalf && !profile::matches_process(active, process) {
        return Err(ChatFailure::NotAGame {
            process: process.to_string(),
            game: active.display_name.clone(),
        });
    }
    Ok(active.clone())
}

/// 挑频道（ADR 041 D1）：缺省 = 游戏配置的默认频道。要的频道没有时**发给更多人不退、发给更少人可以退**：
/// 要“队伍”而没有 → 默认频道（单频道的游戏都是这样）；要“所有人”或别的而没有 → 不发。
/// 返回 (频道, 是否因为 `for_game` 不是这个游戏而退回了默认频道)。
pub fn pick_channel(
    game: &GameProfile,
    pick: Option<&ChannelPick>,
) -> Result<(ChannelStrategy, bool), ChatFailure> {
    let default = game.channel_strategy(None);
    let Some(pick) = pick else {
        return Ok((default, false));
    };
    if pick.for_game.as_deref().is_some_and(|g| g != game.id) {
        // 语音输入：选的频道属于别的游戏（玩家切了游戏）→ 默认频道
        let fell_back = pick.id != default.id;
        return Ok((default, fell_back));
    }
    if let Some(found) = game
        .channels_effective()
        .into_iter()
        .find(|c| c.id == pick.id)
    {
        return Ok((found, false));
    }
    if pick.id == CHANNEL_TEAM && default.id != CHANNEL_ALL {
        return Ok((default, false));
    }
    Err(ChatFailure::NoSuchChannel {
        channel: channel_label(&pick.id).to_string(),
        game: game.display_name.clone(),
    })
}

/// 交给插件的发法时要哪个频道（id）：缺省 = 游戏配置的默认频道；语音输入选的频道属于别的游戏 → 默认频道
/// （和 [`pick_channel`] 前两条一样）。插件那边只有“这里有这个频道就发、没有就不发”（不退，ADR 041 第 2 期）。
/// 返回 (频道 id, 是否因为 `for_game` 不是这个游戏而退回了默认频道)。
pub fn requested_channel(game: &GameProfile, pick: Option<&ChannelPick>) -> (String, bool) {
    let default = game.channel_strategy(None).id;
    match pick {
        None => (default, false),
        Some(pick) if pick.for_game.as_deref().is_some_and(|g| g != game.id) => {
            let fell_back = pick.id != default;
            (default, fell_back)
        }
        Some(pick) => (pick.id.clone(), false),
    }
}

fn channel_label(id: &str) -> &str {
    match id {
        CHANNEL_TEAM => "队伍",
        CHANNEL_ALL => "所有人",
        other => other,
    }
}

/// 估一行要多久（用来判断截止时间前来不来得及）：按键时长、配置里的延时、打字、挡键尾巴、余量。
pub fn estimate_line(profile: &GameProfile, text: &str) -> Duration {
    let mut ms = LINE_TAIL_MS + LINE_MARGIN_MS;
    if !profile.open_chat_key.trim().is_empty() {
        ms += KEY_PRESS_MS + u64::from(profile.pre_open_delay_ms);
    }
    if profile.prefer_clipboard_paste {
        ms += u64::from(profile.pre_paste_delay_ms) + KEY_PRESS_MS + CLIPBOARD_RESTORE_MS;
    } else {
        // 64 个 UTF-16 单元一批 SendInput
        ms += (text.encode_utf16().count() as u64).div_ceil(64) * 5;
    }
    if !profile.send_key.trim().is_empty() {
        ms += u64::from(profile.pre_send_delay_ms) + KEY_PRESS_MS;
    }
    Duration::from_millis(ms)
}

#[cfg(test)]
mod tests;
