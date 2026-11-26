//! 海克斯三选一（ADR 043 D7，原来贴在三张卡片上面的海克斯评分覆盖层）：清单 `[[pushes]]` 的 `augment-pick`。
//!
//! - **看画面**：海斗对局里截游戏画面中间那块识别文字（宿主的 `screen_text`，**固定用本地文字识别**，和全局的识别设置
//!   无关），认出三张海克斯卡片的名字（aramhex-data 的海克斯名），按这局自己英雄的 英雄 × 海克斯 胜率、照 AramHex 的规则
//!   （[`rules::evaluate`]）给每张一个判断。卡片在屏幕上时每秒看一次（本地识别一次 0.1–0.3 秒），不在时每 2 秒。
//! - **一行三格 + 语音**：左 / 中 / 右各一格：首选 / 可选 / 可刷新 / 没数据（认不出的那张写“没认出”）。一次三选一一个
//!   key：刷新了哪张就原地换哪一格（同一个 key 再推，宿主原地更新）；她在卡片刚出现时说一次（“中间那个”，几个说法轮着用），
//!   刷新后最好的那张变了才再说（说法跟着结论走：结论没变，说法就没变，宿主不再说）。三张都不好时说“都一般，可以刷新”。
//!   同样的画面连着看到两次才推（刷新的动画里只认出一部分时不推，免得说错）。卡片没了 [`CARD_GONE`] 就收起
//!   （`push_done`）；之后再出三选一是新的一次（新的 key）。
//! - **本地识别用不了时**：一个本地模型都没装、玩家也没说过“不用”——推一张卡片问要不要下载（页面
//!   `ui://kotone/lol/augment-ocr`，下载 / 不用；海斗选英雄时就问，进了游戏点不了）；下载走本体的模型下载，不改全局的
//!   文字识别选择。用不了（没有模型又说了不用、下载失败、游戏是独占全屏、识别出错）时推一行说清楚原因，**每局最多一次**；
//!   游戏不在前台不算用不了（不截，等它回到前台）。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kotone_core::log;
use kotone_plugin_api::{
    Host, LocalOcr, Push, PushCard, PushCell, PushLine, PushOutcome, PushTone, ScreenText,
    ToolResult,
};
use serde_json::{json, Value};

use super::rules::{self, CardVerdict, Verdict};
use super::{now_ms, PushState, PAGE_AUGMENT_OCR, PUSH_AUGMENT_PICK};
use crate::stats::aramhex;
use crate::LolPlugin;

const GAME_EXE: &str = "League of Legends.exe";
/// 三选一的卡片在画面中间：只截这一块（比例）。
pub(crate) const CARD_REGION: [f64; 4] = [0.12, 0.12, 0.76, 0.62];
/// 卡片在屏幕上时多久看一次（本地识别一次约 0.1–0.3 秒）。
pub(crate) const CARD_POLL: Duration = Duration::from_secs(1);
/// 没有卡片时多久看一次（卡片出来以后最多晚这么久认出来）；不在海斗对局里时也这么久看一眼。
pub(crate) const IDLE_POLL: Duration = Duration::from_secs(2);
/// 卡片没了多久以后收起这一行。
pub(crate) const CARD_GONE: Duration = Duration::from_secs(4);
/// 后台主循环（每 2 秒）多久没说“在海斗对局里”就当这局结束了。
const LIVE_STALE: Duration = Duration::from_secs(8);
/// 截到纯色画面（独占全屏）连着几次才算用不了（切窗口的一瞬间也是黑的）。
const FULLSCREEN_FRAMES: u32 = 3;
/// 插件设置里记“不用下载”的那一项（`plugins.lol.settings.augmentOcrDeclined`）。
pub(crate) const SETTING_DECLINED: &str = "augmentOcrDeclined";
/// 问要不要下载的卡片：一个固定的 key（问过一次，内容没变时宿主什么都不做）。
const OCR_CARD_KEY: &str = "augment-ocr";
/// 那张卡片的名字。
pub(crate) const OCR_CARD_TITLE: &str = "文字识别模型";
/// 左 / 中 / 右（她说的话里的位置）。
const PLACES: [&str; 3] = ["左边", "中间", "右边"];

// ---------- 认卡片 ----------

fn norm(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !c.is_whitespace() && !c.is_ascii_punctuation() && !"·•“”‘’：:「」【】".contains(*c)
        })
        .collect()
}

/// 识别出来的几行字 → 认出来的海克斯（行、id）。名字整行对上，或者一行里包含名字（至少 3 个字）。
/// 同一个海克斯只认一次（取最靠上的那行）。从左到右排。
pub fn match_augments(lines: &[ScreenText], d: &aramhex::Dataset) -> Vec<(ScreenText, i64)> {
    let names: Vec<(i64, String)> = d
        .augments
        .iter()
        .map(|(id, a)| (*id, norm(&a.name)))
        .filter(|(_, n)| n.chars().count() >= 2)
        .collect();
    let mut out: Vec<(ScreenText, i64)> = Vec::new();
    for line in lines {
        let text = norm(&line.text);
        if text.chars().count() < 2 {
            continue;
        }
        let hit = names.iter().find(|(_, n)| *n == text).or_else(|| {
            // 最长的名字优先（“鼓声暴起 低音轰鸣”不要被“鼓声暴起”抢走）
            let mut contained: Vec<&(i64, String)> = names
                .iter()
                .filter(|(_, n)| n.chars().count() >= 3 && text.contains(n.as_str()))
                .collect();
            contained.sort_by_key(|(_, n)| std::cmp::Reverse(n.chars().count()));
            contained.into_iter().next()
        });
        if let Some((id, _)) = hit {
            match out.iter_mut().find(|(_, existing)| existing == id) {
                Some(slot) if slot.0.rect[1] > line.rect[1] => slot.0 = line.clone(),
                Some(_) => {}
                None => out.push((line.clone(), *id)),
            }
        }
    }
    out.sort_by(|a, b| a.0.rect[0].total_cmp(&b.0.rect[0]));
    out
}

/// 名字那一行在第几格（左 / 中 / 右）：按它在游戏画面里的横向中点（三张卡片大约在 30%、50%、70%）。
fn slot_of(line: &ScreenText) -> usize {
    let center = line.rect[0] + line.rect[2] / 2.0;
    if center < 0.4 {
        0
    } else if center < 0.6 {
        1
    } else {
        2
    }
}

/// 这一次三选一现在的样子：三格各是什么判断（None = 这一格没认出来），首选在第几格，是不是三张都不好。
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub slots: [Option<Verdict>; 3],
    pub best: Option<usize>,
    pub weak: bool,
}

impl View {
    /// 认出来的卡片（从左到右）和它们的判断（顺序一样）。
    pub fn new(found: &[(ScreenText, i64)], cards: &[CardVerdict]) -> Self {
        let mut places: Vec<usize> = found.iter().map(|(line, _)| slot_of(line)).collect();
        let mut distinct = places.clone();
        distinct.dedup();
        if distinct.len() != places.len() && found.len() == 3 {
            // 两张落进了同一格（窗口比例怪）：三张都认出来了就照从左到右
            places = vec![0, 1, 2];
        }
        let mut slots = [None; 3];
        for (place, card) in places.iter().zip(cards) {
            slots[*place] = Some(card.verdict);
        }
        Self {
            slots,
            best: slots.iter().position(|s| *s == Some(Verdict::Best)),
            weak: rules::all_weak(cards),
        }
    }

    /// 一行三格：首选（好）/ 可选（可以）/ 可刷新（不好）/ 没数据 / 没认出。
    pub fn line(&self) -> PushLine {
        let cell = |text: &str, tone: Option<PushTone>| PushCell {
            text: text.into(),
            tone,
        };
        PushLine {
            cells: self
                .slots
                .iter()
                .map(|slot| match slot {
                    Some(Verdict::Best) => cell("首选", Some(PushTone::Good)),
                    Some(Verdict::Ok) => cell("可选", Some(PushTone::Ok)),
                    Some(Verdict::Reroll) => cell("可刷新", Some(PushTone::Bad)),
                    Some(Verdict::NoData) => cell("没数据", None),
                    None => cell("没认出", None),
                })
                .collect(),
        }
    }

    /// 她说的那句（几个说法，宿主轮着用）：首选在哪一格；三张都不好时说可以刷新。一张有数据的都没有时不说。
    /// 说法只跟结论走（首选在哪、是不是都不好）：刷新以后结论没变，说法就一样，宿主不再说。
    pub fn voice(&self) -> Vec<String> {
        let Some(best) = self.best else {
            return Vec::new();
        };
        let place = PLACES[best];
        if self.weak {
            vec![
                format!("都一般，可以刷新，非要选就{place}"),
                format!("这三张都不太好，刷新吧，不然拿{place}的"),
            ]
        } else {
            vec![
                format!("{place}那个"),
                format!("选{place}那张"),
                format!("拿{place}的"),
            ]
        }
    }
}

/// 画面上认出来的三选一（认出至少两张才算）。
pub fn view_of(lines: &[ScreenText], d: &aramhex::Dataset, hero: i64) -> Option<View> {
    let found = match_augments(lines, d);
    if found.len() < 2 {
        return None;
    }
    let ids: Vec<i64> = found.iter().map(|(_, id)| *id).collect();
    Some(View::new(&found, &rules::evaluate(d, hero, &ids)))
}

// ---------- 一帧一帧地看 ----------

/// 这一次看画面看到了什么。
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    /// 看到了三选一。
    Cards(View),
    /// 看了，没有三选一。
    Nothing,
    /// 没看成（游戏不在前台、识别出错）：什么都不改。
    Unknown,
}

/// 要做的事。
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// 推（新的一次三选一，或者同一次里变了）。
    Push { key: String, view: View },
    /// 这一次三选一结束了（卡片没了、这局结束了）：收起那一行。
    Done { key: String },
}

#[derive(Debug, Clone)]
struct Open {
    key: String,
    /// 推出去的样子。
    shown: View,
    /// 最近一次看到卡片的时刻。
    seen: Instant,
}

/// 三选一一帧一帧的决定（纯逻辑，时间由调用方给）。
#[derive(Debug, Default)]
pub struct Tracker {
    open: Option<Open>,
    /// 上一帧看到、还没推的样子：同样的连着看到两次才推。
    pending: Option<View>,
}

impl Tracker {
    /// 现在有一次三选一开着（卡片在屏幕上，或者刚没了还不到 [`CARD_GONE`]）。
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// 看了一帧：要推什么、收起什么。`now_ms` 和 `game` 只用来起新的 key（`augment:<对局 id>:<毫秒>`）。
    pub fn frame(&mut self, frame: Frame, now: Instant, now_ms: i64, game: i64) -> Vec<Step> {
        match frame {
            Frame::Unknown => Vec::new(),
            Frame::Nothing => {
                self.pending = None;
                match &self.open {
                    Some(open) if now.saturating_duration_since(open.seen) >= CARD_GONE => {
                        let key = open.key.clone();
                        self.open = None;
                        vec![Step::Done { key }]
                    }
                    _ => Vec::new(),
                }
            }
            Frame::Cards(view) => {
                if let Some(open) = self.open.as_mut() {
                    open.seen = now;
                }
                let stable = self.pending.as_ref() == Some(&view);
                self.pending = Some(view.clone());
                if !stable {
                    return Vec::new();
                }
                match self.open.as_mut() {
                    None => {
                        let key = format!("augment:{game}:{now_ms}");
                        self.open = Some(Open {
                            key: key.clone(),
                            shown: view.clone(),
                            seen: now,
                        });
                        vec![Step::Push { key, view }]
                    }
                    Some(open) if open.shown != view => {
                        open.shown = view.clone();
                        vec![Step::Push {
                            key: open.key.clone(),
                            view,
                        }]
                    }
                    Some(_) => Vec::new(),
                }
            }
        }
    }

    /// 这局结束了、推送关了：开着的那一次收起。
    pub fn close(&mut self) -> Option<Step> {
        self.pending = None;
        self.open.take().map(|open| Step::Done { key: open.key })
    }
}

// ---------- 本地识别用不了 ----------

/// 截图识别为什么没成。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trouble {
    /// 游戏不在前台、最小化了：不截，等它回来（不算用不了）。
    Away,
    /// 截到纯色画面：游戏是独占全屏。
    Fullscreen,
    /// 本地模型没了（删了）。
    NoModel,
    /// 别的（识别出错、截图失败）。
    Failed,
}

/// 宿主 `screen_text` 的错 → 为什么没成。
pub fn trouble(error: &str) -> Trouble {
    if error.starts_with("前台")
        || error.contains("前台窗口")
        || error.starts_with("游戏窗口")
        || error.starts_with("拿不到游戏窗口")
    {
        Trouble::Away
    } else if error.contains("纯色画面") {
        Trouble::Fullscreen
    } else if error.contains("没有装好的本地文字识别模型") {
        Trouble::NoModel
    } else {
        Trouble::Failed
    }
}

/// 用不了时那一行的第二格（第一格是“海克斯评分用不了”）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// 没装本地模型，玩家说了不用下载。
    NoModel,
    /// 下载失败了。
    DownloadFailed,
    /// 游戏是独占全屏。
    Fullscreen,
    /// 识别出错。
    Failed,
}

impl Reason {
    fn text(self) -> &'static str {
        match self {
            Reason::NoModel => "没有本地文字识别模型",
            Reason::DownloadFailed => "识别模型没下成",
            Reason::Fullscreen => "游戏要改成无边框",
            Reason::Failed => "文字识别出错",
        }
    }

    /// 那一行：两格，第一格“不好”的颜色。
    pub fn line(self) -> PushLine {
        PushLine {
            cells: vec![
                PushCell {
                    text: "海克斯评分用不了".into(),
                    tone: Some(PushTone::Bad),
                },
                PushCell {
                    text: self.text().into(),
                    tone: None,
                },
            ],
        }
    }
}

/// 问要不要下载的那张卡片走到哪了。
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Download {
    /// 没问过（或者问了还没答）。
    #[default]
    Idle,
    /// 正在下。
    Running,
    /// 下好了。
    Done,
    /// 没下成（原因）。
    Failed(String),
}

/// 这局（海斗、局内接口有游戏时间）：主循环每一轮写一次。
#[derive(Debug, Clone)]
pub(crate) struct LiveMayhem {
    pub(crate) game_id: i64,
    /// 局内接口里自己英雄的 alias（`Irelia`）。
    pub(crate) alias: String,
    pub(crate) at: Instant,
}

#[derive(Debug, Default)]
pub(crate) struct PickState {
    pub(crate) live: Option<LiveMayhem>,
    tracker: Tracker,
    /// 本地识别确认过能用（识别时发现模型没了就清掉，重新看）。
    ready: bool,
    /// 这次启动推过“要不要下载”的卡片了（卡片在聊天里，不再推）。
    asked: bool,
    download: Download,
    /// 推过“用不了”那一行的对局。
    reason_for: Option<i64>,
    /// 连着截到纯色画面几次了。
    black: u32,
    /// 识别出错已经记过日志（这次启动）。
    logged: bool,
}

/// 问要不要下载的那张卡片的数据（页面 `ui://kotone/lol/augment-ocr`）：`state` = ask / downloading / ready / failed /
/// declined；模型的名字、约多少 MB；没下成时的原因。
pub fn ocr_card(state: &str, name: &str, bytes: u64, reason: Option<&str>) -> Value {
    let mut card = json!({
        "state": state,
        "model": name,
        "sizeMb": (bytes as f64 / 1_000_000.0).round() as u64,
    });
    if let Some(reason) = reason {
        card["reason"] = json!(reason);
    }
    card
}

fn declined(host: &dyn Host) -> bool {
    host.settings()[SETTING_DECLINED].as_bool() == Some(true)
}

fn pushed(what: &str, result: Result<PushOutcome, String>) -> bool {
    match result {
        Ok(PushOutcome::Posted | PushOutcome::Updated | PushOutcome::Unchanged) => true,
        Ok(PushOutcome::Off) => false,
        Err(e) => {
            log::log(&format!("lol: 推送「{what}」发不出去：{e}"));
            false
        }
    }
}

/// 推 / 更新问要不要下载的那张卡片。
fn push_ocr_card(host: &dyn Host, card: Value) -> bool {
    pushed(
        OCR_CARD_TITLE,
        host.push(Push {
            trigger: PUSH_AUGMENT_PICK.into(),
            key: OCR_CARD_KEY.into(),
            card: Some(PushCard {
                title: OCR_CARD_TITLE.into(),
                uri: PAGE_AUGMENT_OCR.into(),
                params: Value::Null,
                data: card,
            }),
            ..Push::default()
        }),
    )
}

impl PickState {
    /// 这局要不要推“用不了”那一行（每局最多一次）：要的话记下。
    fn reason_due(&mut self, game: i64) -> bool {
        if self.reason_for == Some(game) {
            return false;
        }
        self.reason_for = Some(game);
        true
    }
}

impl LolPlugin {
    /// 后台主循环每一轮：告诉三选一的循环现在是不是在海斗对局里（局内接口有游戏时间）、自己是什么英雄。
    pub(crate) fn note_live_game(
        &self,
        host: &dyn Host,
        phase: &str,
        live: Option<&crate::client::live::LiveGame>,
    ) {
        let game = live
            .filter(|g| g.time > 0.0 && crate::client::in_game_phase(phase))
            .filter(|_| self.current_queue() == Some(super::MAYHEM_QUEUE));
        let note = game.map(|g| LiveMayhem {
            game_id: self.current_game_id().unwrap_or(0),
            alias: g.me().map(|p| p.alias.clone()).unwrap_or_default(),
            at: Instant::now(),
        });
        let step = {
            let mut p = self.pushes.lock().unwrap();
            p.pick.live = note;
            if p.pick.live.is_none() {
                p.pick.tracker.close()
            } else {
                None
            }
        };
        if let Some(Step::Done { key }) = step {
            host.push_done(PUSH_AUGMENT_PICK, &key);
        }
    }

    /// 三选一的循环（`watch` 里开，一直跑）：海斗对局里卡片在屏幕上时每秒看一次画面，不在时每 2 秒；不在海斗对局里时
    /// 每 2 秒看一眼。
    pub(crate) fn watch_augment_picks(self: Arc<Self>, host: Arc<dyn Host>) {
        let task_host = host.clone();
        host.spawn(Box::pin(async move {
            loop {
                let wait = self.pick_tick(&task_host).await;
                tokio::time::sleep(wait).await;
            }
        }));
    }

    /// 一轮；返回多久以后再看。
    async fn pick_tick(&self, host: &Arc<dyn Host>) -> Duration {
        let now = Instant::now();
        let live = {
            let mut p = self.pushes.lock().unwrap();
            if p.pick
                .live
                .as_ref()
                .is_some_and(|l| now.saturating_duration_since(l.at) >= LIVE_STALE)
            {
                p.pick.live = None;
            }
            p.pick.live.clone()
        };
        let Some(live) = live.filter(|_| host.push_enabled(PUSH_AUGMENT_PICK)) else {
            // 不在海斗对局里（或者这条推送关着）：开着的那一行收起，不截图
            let step = self.pushes.lock().unwrap().pick.tracker.close();
            if let Some(Step::Done { key }) = step {
                host.push_done(PUSH_AUGMENT_PICK, &key);
            }
            return IDLE_POLL;
        };
        if !self.local_ocr_ready(host, live.game_id) {
            return IDLE_POLL;
        }
        let frame = match host.screen_text(GAME_EXE, CARD_REGION, true).await {
            Ok(lines) => {
                self.pushes.lock().unwrap().pick.black = 0;
                match self.offer_view(host.as_ref(), &lines, &live.alias).await {
                    Some(view) => Frame::Cards(view),
                    None => Frame::Nothing,
                }
            }
            Err(e) => {
                self.screen_failed(host.as_ref(), live.game_id, &e);
                Frame::Unknown
            }
        };
        let steps =
            self.pushes
                .lock()
                .unwrap()
                .pick
                .tracker
                .frame(frame, now, now_ms(), live.game_id);
        for step in steps {
            match step {
                Step::Push { key, view } => {
                    pushed(
                        "海克斯三选一",
                        host.push(Push {
                            trigger: PUSH_AUGMENT_PICK.into(),
                            key,
                            line: Some(view.line()),
                            voice: view.voice(),
                            ..Push::default()
                        }),
                    );
                }
                Step::Done { key } => host.push_done(PUSH_AUGMENT_PICK, &key),
            }
        }
        if self.pushes.lock().unwrap().pick.tracker.is_open() {
            CARD_POLL
        } else {
            IDLE_POLL
        }
    }

    /// 画面上的三选一（这局自己的英雄、aramhex-data 的数据都有时才认得出）。
    async fn offer_view(&self, host: &dyn Host, lines: &[ScreenText], alias: &str) -> Option<View> {
        let hex = self.stats_for(host).aramhex().await.ok()?;
        let hero = self.loaded_data()?.champion_by_alias(alias)?.id;
        view_of(lines, &hex, hero)
    }

    /// 本地识别能不能用；不能用时照情况问一句要不要下载、或者推一行原因（每局最多一次）。
    fn local_ocr_ready(&self, host: &Arc<dyn Host>, game: i64) -> bool {
        if self.pushes.lock().unwrap().pick.ready {
            return true;
        }
        match host.local_ocr() {
            LocalOcr::Ready { .. } => {
                self.pushes.lock().unwrap().pick.ready = true;
                true
            }
            LocalOcr::Missing { .. } => {
                let download = self.pushes.lock().unwrap().pick.download.clone();
                match download {
                    Download::Running => {}
                    Download::Failed(_) => {
                        self.reason_line(host.as_ref(), game, Reason::DownloadFailed)
                    }
                    _ if declined(host.as_ref()) => {
                        self.reason_line(host.as_ref(), game, Reason::NoModel)
                    }
                    _ => self.ask_for_local_ocr(host.as_ref()),
                }
                false
            }
            LocalOcr::Failed { reason } => {
                self.log_once(&format!("lol: 海克斯评分用不了本地文字识别：{reason}"));
                self.reason_line(host.as_ref(), game, Reason::Failed);
                false
            }
        }
    }

    /// 截图识别没成：游戏不在前台不算；独占全屏连着几次、识别出错时推一行原因（每局最多一次）。
    fn screen_failed(&self, host: &dyn Host, game: i64, error: &str) {
        let reason = {
            let mut p = self.pushes.lock().unwrap();
            match trouble(error) {
                Trouble::Away => None,
                Trouble::Fullscreen => {
                    p.pick.black += 1;
                    (p.pick.black >= FULLSCREEN_FRAMES).then_some(Reason::Fullscreen)
                }
                Trouble::NoModel => {
                    p.pick.ready = false;
                    None
                }
                Trouble::Failed => Some(Reason::Failed),
            }
        };
        if let Some(reason) = reason {
            self.log_once(&format!("lol: 海克斯评分看不了画面：{error}"));
            self.reason_line(host, game, reason);
        }
    }

    fn log_once(&self, text: &str) {
        let first = !std::mem::replace(&mut self.pushes.lock().unwrap().pick.logged, true);
        if first {
            log::log(text);
        }
    }

    /// 用不了：推一行原因（这局推过就不再推）。
    fn reason_line(&self, host: &dyn Host, game: i64, reason: Reason) {
        if !self.pushes.lock().unwrap().pick.reason_due(game) {
            return;
        }
        pushed(
            "海克斯三选一",
            host.push(Push {
                trigger: PUSH_AUGMENT_PICK.into(),
                key: format!("augment-off:{game}"),
                line: Some(reason.line()),
                ..Push::default()
            }),
        );
    }

    /// 一个本地模型都没装、玩家没说过不用、这次启动还没问过：推一张卡片问要不要下载（海斗选英雄时、进了海斗对局时）。
    pub(crate) fn ask_for_local_ocr(&self, host: &dyn Host) {
        {
            let p = self.pushes.lock().unwrap();
            if p.pick.asked || p.pick.download != Download::Idle {
                return;
            }
        }
        if !host.push_enabled(PUSH_AUGMENT_PICK) || declined(host) {
            return;
        }
        let LocalOcr::Missing { name, bytes, .. } = host.local_ocr() else {
            return;
        };
        if push_ocr_card(host, ocr_card("ask", &name, bytes, None)) {
            self.pushes.lock().unwrap().pick.asked = true;
        }
    }

    /// `lol.augment_ocr`（只给页面）：那张卡片上的按钮。`action`：download 下载推荐的本地模型（在后台下，下完更新卡片）、
    /// decline 不用（记进插件设置，以后不再问）、status 现在怎样。返回卡片的数据。
    pub(crate) async fn augment_ocr_tool(
        &self,
        host: &Arc<dyn Host>,
        arguments: &Value,
    ) -> ToolResult {
        match arguments["action"].as_str().unwrap_or("status") {
            "download" => start_download(self.pushes.clone(), host.clone()),
            "decline" => {
                let mut patch = serde_json::Map::new();
                patch.insert(SETTING_DECLINED.into(), json!(true));
                if let Err(e) = host.update_settings(Value::Object(patch)) {
                    log::log(&format!("lol: 记不下“不用下载识别模型”：{e}"));
                }
                push_ocr_card(host.as_ref(), json!({"state": "declined"}));
            }
            _ => {}
        }
        let card = self.ocr_card_now(host.as_ref());
        let text = match card["state"].as_str().unwrap_or("") {
            "ready" => "本地文字识别模型能用了",
            "downloading" => "正在下载本地文字识别模型",
            "declined" => "玩家说了不用下载本地文字识别模型",
            "failed" => "本地文字识别模型没下成",
            _ => "海克斯评分要用本地文字识别模型，还没下载",
        };
        ToolResult::content(json!({"status": "ok", "text": text})).with_page(PAGE_AUGMENT_OCR, card)
    }

    /// 那张卡片现在该是什么样。
    fn ocr_card_now(&self, host: &dyn Host) -> Value {
        let download = self.pushes.lock().unwrap().pick.download.clone();
        match (host.local_ocr(), download) {
            (LocalOcr::Ready { name, .. }, _) => ocr_card("ready", &name, 0, None),
            (LocalOcr::Missing { name, bytes, .. }, Download::Running) => {
                ocr_card("downloading", &name, bytes, None)
            }
            (LocalOcr::Missing { name, bytes, .. }, Download::Failed(reason)) => {
                ocr_card("failed", &name, bytes, Some(&reason))
            }
            (LocalOcr::Missing { .. }, _) if declined(host) => json!({"state": "declined"}),
            (LocalOcr::Missing { name, bytes, .. }, _) => ocr_card("ask", &name, bytes, None),
            (LocalOcr::Failed { reason }, _) => ocr_card("failed", "", 0, Some(&reason)),
        }
    }
}

/// 在后台下推荐的本地模型（已经在下就什么都不做）：卡片先改成“正在下载”，下完改成“下好了”或“没下成”。
fn start_download(pushes: Arc<Mutex<PushState>>, host: Arc<dyn Host>) {
    let (name, bytes) = match host.local_ocr() {
        LocalOcr::Missing { name, bytes, .. } => (name, bytes),
        _ => return,
    };
    {
        let mut p = pushes.lock().unwrap();
        if p.pick.download == Download::Running {
            return;
        }
        p.pick.download = Download::Running;
    }
    push_ocr_card(host.as_ref(), ocr_card("downloading", &name, bytes, None));
    let task_host = host.clone();
    host.spawn(Box::pin(async move {
        let result = task_host.download_local_ocr().await;
        let card = {
            let mut p = pushes.lock().unwrap();
            match result {
                Ok(()) => {
                    p.pick.download = Download::Done;
                    p.pick.ready = false;
                    ocr_card("ready", &name, 0, None)
                }
                Err(e) => {
                    log::log(&format!("lol: 本地文字识别模型没下成：{e}"));
                    p.pick.download = Download::Failed(e.clone());
                    ocr_card("failed", &name, bytes, Some(&e))
                }
            }
        };
        push_ocr_card(task_host.as_ref(), card);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::aramhex::{parse_latest, Dataset};

    fn dataset() -> Dataset {
        let latest =
            parse_latest(&json!({"path": "patches/16.23/normal", "patch": "16.23"})).unwrap();
        Dataset::from_json(
            &latest,
            &json!({"39": {"games": 100000, "wins": 50000, "win_rate": 0.5}}),
            &json!({
                "1": {"name": "鼓声暴起", "rarity": "gold"},
                "2": {"name": "鼓声暴起 低音轰鸣", "rarity": "prismatic"},
                "3": {"name": "终极九头蛇", "rarity": "prismatic"},
                "4": {"name": "弹跳蜜蜂", "rarity": "silver"},
                "5": {"name": "珠光护手", "rarity": "prismatic"},
                "6": {"name": "冰川之心", "rarity": "gold"}
            }),
            &json!({"39": {
                "2": {"games": 30000, "wins": 17000},
                "3": {"games": 10000, "wins": 5800},
                "4": {"games": 6000, "wins": 2900},
                "5": {"games": 20000, "wins": 12400},
                "6": {"games": 20000, "wins": 9000}
            }}),
        )
    }

    fn line(text: &str, x: f64, y: f64) -> ScreenText {
        ScreenText {
            text: text.into(),
            rect: [x, y, 0.1, 0.03],
        }
    }

    /// 识别出来的一帧：左（x 0.2）、中（0.4）、右（0.6）三张卡片的名字，加上标题和说明文字。
    fn ocr(left: &str, middle: &str, right: &str) -> Vec<ScreenText> {
        vec![
            line("选择一个强化", 0.4, 0.1),
            line(right, 0.6, 0.35),
            line(left, 0.2, 0.35),
            line(middle, 0.4, 0.36),
            line("每次攻击……", 0.4, 0.5),
        ]
    }

    #[test]
    fn three_cards_are_found_and_judged_left_to_right() {
        let d = dataset();
        let lines = ocr("鼓声暴起·低音轰鸣", "弹跳 蜜蜂", "终极九头蛇");
        let found = match_augments(&lines, &d);
        let ids: Vec<i64> = found.iter().map(|(_, id)| *id).collect();
        assert_eq!(ids, vec![2, 4, 3], "从左到右，长名字优先");
        let view = view_of(&lines, &d, 39).unwrap();
        // 首选是终极九头蛇（5800/10000 = 58%，平滑后 +7.8，也比 17000/30000 的 +6.6 高），在最右边；
        // 鼓声暴起 低音轰鸣低不到 2 个点：可选；弹跳蜜蜂比平均低：可刷新
        assert_eq!(
            view.slots,
            [
                Some(Verdict::Ok),
                Some(Verdict::Reroll),
                Some(Verdict::Best)
            ]
        );
        assert_eq!(view.best, Some(2));
        assert!(!view.weak);
        let cells: Vec<(String, Option<PushTone>)> = view
            .line()
            .cells
            .into_iter()
            .map(|c| (c.text, c.tone))
            .collect();
        assert_eq!(
            cells,
            [
                ("可选".to_string(), Some(PushTone::Ok)),
                ("可刷新".to_string(), Some(PushTone::Bad)),
                ("首选".to_string(), Some(PushTone::Good)),
            ]
        );
        assert_eq!(view.voice(), ["右边那个", "选右边那张", "拿右边的"]);
        for v in view.voice() {
            assert!(v.chars().count() <= Push::VOICE_MAX, "{v}");
        }
    }

    #[test]
    fn ordinary_text_is_not_a_card_and_a_missing_card_is_said_so() {
        let d = dataset();
        assert!(
            match_augments(&[line("对面拿了火龙", 0.4, 0.3), line("鼓", 0.1, 0.1)], &d).is_empty()
        );
        assert!(
            view_of(&ocr("对面拿了火龙", "终极九头蛇", "看不清"), &d, 39).is_none(),
            "只认出一张不算"
        );
        // 中间那张认不出（新海克斯、名字被挡住）：那一格写“没认出”，另外两格照常，位置不乱
        let view = view_of(&ocr("鼓声暴起 低音轰鸣", "？？", "终极九头蛇"), &d, 39).unwrap();
        assert_eq!(view.slots, [Some(Verdict::Ok), None, Some(Verdict::Best)]);
        assert_eq!(view.line().cells[1].text, "没认出");
        assert_eq!(view.line().cells[1].tone, None);
    }

    #[test]
    fn when_all_three_are_weak_she_says_reroll() {
        let d = dataset();
        // 弹跳蜜蜂 48.3%、冰川之心 45%：都比平均低；首选还是分最高的那张（非要选就拿它）
        let lines = vec![line("冰川之心", 0.2, 0.35), line("弹跳蜜蜂", 0.4, 0.35)];
        let view = view_of(&lines, &d, 39).unwrap();
        assert_eq!(view.best, Some(1));
        assert!(view.weak);
        assert_eq!(
            view.voice(),
            [
                "都一般，可以刷新，非要选就中间",
                "这三张都不太好，刷新吧，不然拿中间的"
            ]
        );
        for v in view.voice() {
            assert!(v.chars().count() <= Push::VOICE_MAX, "{v}");
        }
        // 一张有数据的都没有：不说
        let none = View {
            slots: [Some(Verdict::NoData), Some(Verdict::NoData), None],
            best: None,
            weak: false,
        };
        assert!(none.voice().is_empty());
        assert_eq!(none.line().cells[0].text, "没数据");
    }

    /// 宿主的规矩（`agent/push.rs`）：同一个 key 再推一次，说法变了才开口。
    fn says(previous: Option<&[String]>, next: &[String]) -> bool {
        !next.is_empty() && previous.is_none_or(|p| p != next)
    }

    /// ADR 043 D7 的一次三选一，一帧一帧：出现 → 一行 + 语音；刷新了一张、首选没变 → 原地换那一格、不说；首选变了 → 说；
    /// 卡片没了 4 秒 → 收起；之后再出来是新的一次（新的 key）。
    #[test]
    fn one_offer_frame_by_frame() {
        let d = dataset();
        let t0 = Instant::now();
        let at = |s: f64| t0 + Duration::from_secs_f64(s);
        let mut tracker = Tracker::default();
        let mut pushes: Vec<(String, PushLine, Vec<String>)> = Vec::new();
        let mut done: Vec<String> = Vec::new();
        let frame_of = |lines: Option<Vec<ScreenText>>| match lines {
            None => Frame::Unknown,
            Some(lines) => match view_of(&lines, &d, 39) {
                Some(view) => Frame::Cards(view),
                None => Frame::Nothing,
            },
        };
        // 看一帧（第 s 秒）：推的记进 pushes，收起的记进 done
        macro_rules! run {
            ($tracker:expr, $lines:expr, $s:expr) => {
                for step in $tracker.frame(frame_of($lines), at($s), ($s * 1000.0) as i64, 9100001)
                {
                    match step {
                        Step::Push { key, view } => pushes.push((key, view.line(), view.voice())),
                        Step::Done { key } => done.push(key),
                    }
                }
            };
        }
        let first = ocr("鼓声暴起 低音轰鸣", "弹跳蜜蜂", "终极九头蛇");
        // 出现：第一帧先记下，第二帧一样才推（一行三格 + 说“右边那个”）
        run!(&mut tracker, Some(first.clone()), 0.0);
        assert!(pushes.is_empty(), "只看到一次，不推");
        run!(&mut tracker, Some(first.clone()), 1.0);
        assert_eq!(pushes.len(), 1);
        let key = pushes[0].0.clone();
        assert_eq!(key, "augment:9100001:1000");
        assert!(says(None, &pushes[0].2));
        assert!(tracker.is_open());
        // 一样的画面：不再推
        run!(&mut tracker, Some(first.clone()), 2.0);
        assert_eq!(pushes.len(), 1);
        // 刷新了中间那张（弹跳蜜蜂 → 鼓声暴起：没数据也不比首选好）：刷新的动画那一帧只认出两张，不推；
        // 稳定以后推一次，同一个 key，只有中间那格变了，首选还在右边 → 说法一样（宿主不再说）
        run!(
            &mut tracker,
            Some(ocr("鼓声暴起 低音轰鸣", "……", "终极九头蛇")),
            3.0
        );
        let refreshed = ocr("鼓声暴起 低音轰鸣", "鼓声暴起", "终极九头蛇");
        run!(&mut tracker, Some(refreshed.clone()), 4.0);
        assert_eq!(pushes.len(), 1, "动画里的一帧和下一帧不一样：都不推");
        run!(&mut tracker, Some(refreshed.clone()), 5.0);
        assert_eq!(pushes.len(), 2);
        assert_eq!(pushes[1].0, key, "同一次三选一：同一个 key（原地更新）");
        assert_eq!(pushes[1].1.cells[0], pushes[0].1.cells[0]);
        assert_eq!(pushes[1].1.cells[2], pushes[0].1.cells[2]);
        assert_ne!(pushes[1].1.cells[1], pushes[0].1.cells[1]);
        assert!(!says(Some(&pushes[0].2), &pushes[1].2), "首选没变：不说");
        // 刷新了右边那张（终极九头蛇 → 珠光护手 62%）：首选还在右边，但比它低得多的左边那张成了可刷新 → 原地换两格、不说
        let r2 = ocr("鼓声暴起 低音轰鸣", "鼓声暴起", "珠光护手");
        run!(&mut tracker, Some(r2.clone()), 6.0);
        run!(&mut tracker, Some(r2.clone()), 7.0);
        assert_eq!(pushes.len(), 3);
        assert_eq!(pushes[2].0, key);
        assert_eq!(pushes[2].1.cells[0].text, "可刷新");
        assert_eq!(pushes[2].1.cells[2].text, "首选");
        assert!(
            !says(Some(&pushes[1].2), &pushes[2].2),
            "首选还在右边：不说"
        );
        // 再刷新左边、右边：左边成了首选 → 说“左边那个”
        let r3 = ocr("珠光护手", "鼓声暴起", "冰川之心");
        run!(&mut tracker, Some(r3.clone()), 8.0);
        run!(&mut tracker, Some(r3.clone()), 9.0);
        assert_eq!(pushes.len(), 4);
        assert_eq!(pushes[3].0, key);
        assert_eq!(pushes[3].2[0], "左边那个");
        assert!(says(Some(&pushes[2].2), &pushes[3].2), "首选变了：说");
        // 游戏切走了一下（没截成）：什么都不改
        run!(&mut tracker, None, 10.0);
        assert!(tracker.is_open());
        // 选完了，卡片没了：不到 4 秒不收（识别偶尔漏一帧），到了 4 秒收起
        run!(&mut tracker, Some(Vec::new()), 11.0);
        run!(&mut tracker, Some(Vec::new()), 12.0);
        assert!(done.is_empty());
        run!(&mut tracker, Some(Vec::new()), 13.0);
        assert_eq!(done, std::slice::from_ref(&key));
        assert!(!tracker.is_open());
        // 过一会儿又出三选一：新的一次，新的 key
        run!(&mut tracker, Some(first.clone()), 300.0);
        run!(&mut tracker, Some(first.clone()), 301.0);
        assert_eq!(pushes.len(), 5);
        assert_eq!(pushes[4].0, "augment:9100001:301000");
        assert_ne!(pushes[4].0, key);
        assert!(says(None, &pushes[4].2));
        // 这局结束了：开着的那一次收起
        assert_eq!(
            tracker.close(),
            Some(Step::Done {
                key: "augment:9100001:301000".into()
            })
        );
    }

    /// 清单（ADR 043 D4、D7）：三选一是一行 + 语音 + 卡片（问下载的那张）、要紧、要“用网上的统计”，那一行在悬浮窗里一直留到
    /// `push_done`（卡片没了）；选英雄只有卡片、要客户端和统计。
    #[test]
    fn the_manifest_declares_both_pushes() {
        use kotone_plugin_api::{PushForm, PushPriority};
        let pick = crate::manifest().push_spec(PUSH_AUGMENT_PICK).unwrap();
        assert!(pick.until_done, "一行留到卡片没了");
        assert_eq!(pick.priority, PushPriority::High);
        assert_eq!(
            pick.forms,
            [PushForm::Line, PushForm::Voice, PushForm::Card]
        );
        assert_eq!(pick.needs, ["stats"]);
        let select = crate::manifest()
            .push_spec(super::super::PUSH_CHAMP_SELECT)
            .unwrap();
        assert_eq!(select.forms, [PushForm::Card]);
        assert_eq!(select.needs, ["client", "stats"]);
        assert!(!select.until_done);
    }

    #[test]
    fn screen_errors_are_sorted_out() {
        assert_eq!(trouble("前台不是 League of Legends.exe"), Trouble::Away);
        assert_eq!(
            trouble("前台是 Kotone 自己的窗口：先用 game.focus 切到游戏"),
            Trouble::Away
        );
        assert_eq!(
            trouble("当前没有前台窗口：先用 game.focus 切到游戏"),
            Trouble::Away
        );
        assert_eq!(trouble("游戏窗口客户区为空（可能最小化了）"), Trouble::Away);
        assert_eq!(
            trouble("截到的是纯色画面，游戏可能是独占全屏：请改成无边框窗口"),
            Trouble::Fullscreen
        );
        assert_eq!(trouble("没有装好的本地文字识别模型"), Trouble::NoModel);
        assert_eq!(trouble("文字识别失败：onnx"), Trouble::Failed);
        for reason in [
            Reason::NoModel,
            Reason::DownloadFailed,
            Reason::Fullscreen,
            Reason::Failed,
        ] {
            let line = reason.line();
            assert_eq!(line.cells.len(), 2);
            assert!(line
                .cells
                .iter()
                .all(|c| c.text.chars().count() <= PushCell::MAX_CHARS));
        }
    }

    /// 只记推送和插件设置的假宿主（本地识别的状态、下载结果由测试给）。
    struct FakeHost {
        ocr: Mutex<LocalOcr>,
        settings: Mutex<Value>,
        pushes: Mutex<Vec<Push>>,
        enabled: bool,
    }

    impl FakeHost {
        fn new(ocr: LocalOcr) -> Arc<Self> {
            Arc::new(Self {
                ocr: Mutex::new(ocr),
                settings: Mutex::new(json!({})),
                pushes: Mutex::new(Vec::new()),
                enabled: true,
            })
        }
        fn pushed(&self) -> Vec<Push> {
            self.pushes.lock().unwrap().clone()
        }
    }

    fn missing() -> LocalOcr {
        LocalOcr::Missing {
            model: "ppocrv5-ch-mobile".into(),
            name: "PP-OCRv5 mobile 中英日（本地，CPU）".into(),
            bytes: 21_450_882,
        }
    }

    impl Host for FakeHost {
        fn plugin(&self) -> &str {
            "lol"
        }
        fn enabled(&self) -> bool {
            true
        }
        fn source_enabled(&self, _source: &str) -> bool {
            true
        }
        fn settings(&self) -> Value {
            self.settings.lock().unwrap().clone()
        }
        fn update_settings(&self, patch: Value) -> Result<(), String> {
            let mut s = self.settings.lock().unwrap();
            for (k, v) in patch.as_object().unwrap() {
                s[k] = v.clone();
            }
            Ok(())
        }
        fn data_dir(&self) -> std::path::PathBuf {
            std::path::PathBuf::new()
        }
        fn spawn(&self, task: kotone_plugin_api::BoxFuture<'static, ()>) {
            tokio::spawn(task);
        }
        fn process_ids(&self, _exe: &str) -> Vec<u32> {
            Vec::new()
        }
        fn process_command_line(&self, _pid: u32) -> Option<String> {
            None
        }
        fn game_observe(&self, _game: &str, _observation: kotone_plugin_api::Observation) {}
        fn game_claim(&self, _game: &str, _claimed: bool) {}
        fn game_event(
            &self,
            _game: &str,
            _kind: &str,
            _source: &str,
            _data: serde_json::Map<String, Value>,
            _game_time: Option<f64>,
        ) {
        }
        fn push(&self, push: Push) -> Result<PushOutcome, String> {
            push.check(crate::manifest())?;
            self.pushes.lock().unwrap().push(push);
            Ok(PushOutcome::Posted)
        }
        fn push_enabled(&self, _trigger: &str) -> bool {
            self.enabled
        }
        fn local_ocr(&self) -> LocalOcr {
            self.ocr.lock().unwrap().clone()
        }
        fn download_local_ocr(&self) -> kotone_plugin_api::BoxFuture<'static, Result<(), String>> {
            *self.ocr.lock().unwrap() = LocalOcr::Ready {
                model: "ppocrv5-ch-mobile".into(),
                name: "PP-OCRv5 mobile".into(),
            };
            Box::pin(async { Ok(()) })
        }
    }

    /// 没有本地模型：第一次问一句（卡片，问过不再问）；玩家点“不用”记进插件设置，以后每局推一行原因、每局最多一次；
    /// 点“下载”卡片先变“正在下载”，下完变“下好了”，之后就能看画面了。
    #[tokio::test]
    async fn no_local_model_asks_once_then_explains_once_per_game() {
        let plugin = LolPlugin::new();
        let fake = FakeHost::new(missing());
        let host: Arc<dyn Host> = fake.clone();
        // 第一次：问要不要下载（卡片，约 21 MB）
        assert!(!plugin.local_ocr_ready(&host, 1));
        assert!(!plugin.local_ocr_ready(&host, 1));
        let asked = fake.pushed();
        assert_eq!(asked.len(), 1, "问过不再问");
        let card = asked[0].card.as_ref().unwrap();
        assert_eq!(asked[0].key, OCR_CARD_KEY);
        assert_eq!(card.uri, PAGE_AUGMENT_OCR);
        assert_eq!(card.data["state"], "ask");
        assert_eq!(card.data["sizeMb"], 21);
        // 玩家点了“不用”：记进插件设置，卡片改成“不用了”
        let result = plugin
            .augment_ocr_tool(&host, &json!({"action": "decline"}))
            .await;
        assert_eq!(fake.settings()[SETTING_DECLINED], true);
        assert_eq!(result.structured_content.unwrap()["state"], "declined");
        assert_eq!(
            fake.pushed()[1].card.as_ref().unwrap().data["state"],
            "declined"
        );
        // 以后：每局一行原因（这局一次，下一局再一次）
        for game in [1, 1, 2, 2] {
            assert!(!plugin.local_ocr_ready(&host, game));
        }
        let lines: Vec<Push> = fake.pushed().into_iter().skip(2).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].key, "augment-off:1");
        assert_eq!(lines[1].key, "augment-off:2");
        let cells = &lines[0].line.as_ref().unwrap().cells;
        assert_eq!(cells[0].text, "海克斯评分用不了");
        assert_eq!(cells[1].text, "没有本地文字识别模型");
        assert!(lines[0].card.is_none() && lines[0].voice.is_empty());
    }

    #[tokio::test]
    async fn the_download_button_downloads_without_asking_again() {
        let plugin = LolPlugin::new();
        let fake = FakeHost::new(missing());
        let host: Arc<dyn Host> = fake.clone();
        plugin.ask_for_local_ocr(host.as_ref());
        let result = plugin
            .augment_ocr_tool(&host, &json!({"action": "download"}))
            .await;
        // 工具马上返回（在后台下）；卡片先“正在下载”，下完“下好了”
        let state = result.structured_content.unwrap()["state"].clone();
        assert!(state == "downloading" || state == "ready", "{state}");
        for _ in 0..50 {
            if fake.pushed().len() >= 3 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let states: Vec<Value> = fake
            .pushed()
            .iter()
            .map(|p| p.card.as_ref().unwrap().data["state"].clone())
            .collect();
        assert_eq!(states, ["ask", "downloading", "ready"]);
        assert!(plugin.local_ocr_ready(&host, 1), "下好了：能看画面了");
        assert_eq!(fake.pushed().len(), 3, "不再问、不推原因");
    }
}
