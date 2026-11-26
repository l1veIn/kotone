//! 悬浮窗现在显示什么、什么时候收（纯状态，不碰窗口；时间用毫秒传进来，便于测试）。ADR 043 D6（2026-11-26 晚修订）。
//!
//! 悬浮窗出现时角上一定是她的头像，旁边是**气泡**（一条简单的：她在听、在想、说了一句短话，推送的一行）或**聊天小窗**
//! （复杂的：卡片、工具调用、长回答、同时来了几条）。旁边放哪个只由界面按内容定（`routes/floating/shape.ts` 的
//! `floatingShape`，规则只写在那一处），界面每次量好大小时报上来（[`Model::shape`]）；这里管显示不显示，和头像外面那一圈
//! **进度条**（[`Ring`]，还剩多久）：
//!
//! - 最后一次变化以后，气泡走 [`BUBBLE_MS`]、聊天小窗走 [`MINI_MS`]；走完整个收起（头像和旁边的一起）。
//! - 变化：她换了状态、说完了；推送来了一行、弹出一张卡片；聊天小窗开着时推送原地更新了；界面从气泡换成了小窗。
//! - 停着不走：鼠标在上面、正拖着、她在听 / 在想 / 在说 / 等你回答（[`crate::bubble::Model::active`]）、写了
//!   `until_done` 的一行插件还没说结束（插件一直不说，最后一次更新后 [`UNTIL_DONE_MAX_MS`] 就不再等）。
//!   鼠标移开接着走，至少还剩 [`AFTER_LEAVE_MS`]（不至于一移开就没了）。
//! - 她说完了（`bubble.rs` 按字数停留后收起她那一行）：这里接着留着她最后那句，直到进度条走完；她还在听 / 在想时被取消
//!   （Esc、呼叫键打断）就马上收起她那一行（没有她说的话要看）。
//! - ×、打开 Kotone：都收起，直到下一条推送；她这一行收到她换了状态。进度条走完收起的，她再开口（在听 / 在想 / 在说）
//!   就再出来。

use kotone_plugin_api::PushCell;
use serde::Serialize;

use crate::bubble::Phase;

/// 气泡：最后一次变化后走多久。
pub const BUBBLE_MS: u64 = 6_000;
/// 聊天小窗：最后一次变化后走多久（东西多，多给一点时间看）。
pub const MINI_MS: u64 = 10_000;
/// 鼠标移开（或拖完）以后至少还剩多久。
pub const AFTER_LEAVE_MS: u64 = 2_000;
/// 写了 `until_done` 的一行，插件一直不说结束时最多等多久（从最后一次更新算）。
pub const UNTIL_DONE_MAX_MS: u64 = 90_000;
/// 气泡里最多留几行推送（再多最旧的先走；两行以上界面就换成聊天小窗了）。
pub const MAX_LINES: usize = 3;

/// 气泡里的一行推送。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    /// `插件:key`（同一条再推一次原地换掉）。
    pub id: String,
    pub plugin: String,
    pub plugin_name: String,
    pub key: String,
    pub conversation_id: String,
    pub cells: Vec<PushCell>,
    /// 一直显示到插件说结束（清单的 `until_done`）。
    pub until_done: bool,
    /// 最后一次更新的时间。
    #[serde(skip)]
    updated: u64,
}

impl Line {
    pub fn new(
        plugin: &str,
        plugin_name: &str,
        key: &str,
        conversation_id: &str,
        cells: Vec<PushCell>,
        until_done: bool,
    ) -> Self {
        Self {
            id: id(plugin, key),
            plugin: plugin.into(),
            plugin_name: plugin_name.into(),
            key: key.trim().into(),
            conversation_id: conversation_id.into(),
            cells,
            until_done,
            updated: 0,
        }
    }

    /// 还在等插件说结束（停着进度条）。
    fn waiting(&self, now: u64) -> bool {
        self.until_done && now < self.updated + UNTIL_DONE_MAX_MS
    }
}

/// 推送的标识（和聊天页一样：插件 + key）。
pub fn id(plugin: &str, key: &str) -> String {
    format!("{plugin}:{}", key.trim())
}

/// 弹出聊天小窗的那条推送（新的卡片、带 alert 的更新）：哪段对话、定位到哪一条。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mini {
    pub conversation_id: String,
    /// 定位到的那一条（`插件:key`）。
    pub focus: String,
    pub key: String,
}

/// 头像旁边放什么（界面定的，[`Model::shape`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    #[default]
    Bubble,
    Mini,
}

/// 头像外面那一圈进度条（给界面画）：`at_ms` 那一刻还剩 `left_ms`；`running` 时从那一刻起往下走。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ring {
    /// 一整圈多长（气泡 [`BUBBLE_MS`]、聊天小窗 [`MINI_MS`]）。
    pub total_ms: u64,
    pub left_ms: u64,
    /// false = 停着（鼠标在上面、拖着、她在忙、等插件说结束）。
    pub running: bool,
    pub at_ms: u64,
}

/// 给界面的样子。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    /// 内容的版本（每变一次加一）。
    pub revision: u64,
    pub shown: bool,
    /// 她这一行在不在（内容是 `kotone://bubble` 的快照；她说完以后界面接着画她最后那句）。
    pub her: bool,
    /// 推送的几行（新的在后面）。
    pub lines: Vec<Line>,
    /// 弹出聊天小窗的那条推送。
    pub mini: Option<Mini>,
    /// 这一次出现的时间（毫秒）：界面拿它分出“这一次来的”消息。
    pub since: u64,
    /// 收着时 None。
    pub ring: Option<Ring>,
}

/// 收起时她在哪个状态。
#[derive(Debug, Clone, Copy, PartialEq)]
struct Closed {
    phase: Phase,
    /// 玩家点的（×、打开 Kotone）：她换了状态才再出来。进度条走完的：她再开口也出来。
    by_player: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Her {
    phase: Phase,
    /// 在听、在想、在说、等你回答（进度条停着）。
    active: bool,
}

/// 进度条的钟：停着时 `from` 是 None、`left` 是剩下的；走着时从 `from` 起 `left` 往下减。
#[derive(Debug, Clone, Copy, Default)]
struct Clock {
    total: u64,
    left: u64,
    from: Option<u64>,
}

impl Clock {
    fn left_at(&self, now: u64) -> u64 {
        match self.from {
            Some(from) => self.left.saturating_sub(now.saturating_sub(from)),
            None => self.left,
        }
    }
}

#[derive(Debug, Default)]
pub struct Model {
    revision: u64,
    /// 她这一行（`bubble.rs` 报的；她说完、`bubble.rs` 收起以后这里还留着最后那句）。
    her: Option<Her>,
    her_closed: Option<Closed>,
    lines: Vec<Line>,
    mini: Option<Mini>,
    shape: Shape,
    /// 上一次算的显示着没有。
    shown: bool,
    since: u64,
    /// 这一次出现时的版本：更早的版本报上来的样子不算。
    since_revision: u64,
    clock: Clock,
    hovering: bool,
    holding: bool,
}

impl Model {
    fn her_visible(&self) -> bool {
        self.her.is_some() && self.her_closed.is_none()
    }

    /// 显示着的她这一行（收起了是 None）。
    fn her_seen(&self) -> Option<Her> {
        self.her.filter(|_| self.her_closed.is_none())
    }

    /// 有东西要显示：她这一行、推送的几行、弹出的卡片，或者界面已经换成了聊天小窗（小窗里的对话就是内容）。
    fn content(&self) -> bool {
        self.her_visible()
            || !self.lines.is_empty()
            || self.mini.is_some()
            || self.shape == Shape::Mini
    }

    fn held(&self, now: u64) -> bool {
        self.hovering
            || self.holding
            || (self.her_visible() && self.her.is_some_and(|h| h.active))
            || self.lines.iter().any(|l| l.waiting(now))
    }

    fn total(&self) -> u64 {
        match self.shape {
            Shape::Bubble => BUBBLE_MS,
            Shape::Mini => MINI_MS,
        }
    }

    /// 内容变了：进度条回到满的一圈（走不走由 [`Self::settle`] 定）。
    fn touch(&mut self, now: u64) {
        let total = self.total();
        self.clock = Clock {
            total,
            left: total,
            from: (self.clock.from.is_some()).then_some(now),
        };
    }

    /// 改完以后：算显示着没有（刚出来记下时间、刚收起清掉样子），按该不该停让进度条停下或接着走。
    fn settle(&mut self, now: u64) {
        let shown = self.content();
        if shown && !self.shown {
            self.since = now;
            self.since_revision = self.revision;
            self.clock = Clock::default();
            self.touch(now);
        }
        if !shown && self.shown {
            self.shape = Shape::Bubble;
            self.clock = Clock::default();
            self.hovering = false;
        }
        self.shown = shown;
        if !shown {
            return;
        }
        let held = self.held(now);
        match (held, self.clock.from) {
            (true, Some(_)) => {
                self.clock.left = self.clock.left_at(now);
                self.clock.from = None;
            }
            (false, None) => {
                self.clock.left = self.clock.left.max(AFTER_LEAVE_MS);
                self.clock.from = Some(now);
            }
            _ => {}
        }
    }

    fn bump(&mut self) -> bool {
        self.revision += 1;
        true
    }

    /// 她这一行（`bubble.rs` 每次改状态都报；`None` = 那边收起了）。`active`：她在听 / 在想 / 在说 / 等你回答。
    /// 返回界面要不要变。
    pub fn her(&mut self, phase: Option<Phase>, active: bool, now: u64) -> bool {
        let before = self.her_seen();
        match phase {
            Some(phase) => {
                if let Some(closed) = self.her_closed {
                    if phase != closed.phase || (!closed.by_player && active) {
                        self.her_closed = None;
                    }
                }
                self.her = Some(Her { phase, active });
            }
            None => {
                // 收起过的不再拿出来；她说完了的留着最后那句（进度条走完再收）；还在忙时被取消的马上收
                let closed = self.her_closed.take().is_some();
                self.her = match self.her {
                    Some(her) if !her.active && !closed => Some(her),
                    _ => None,
                };
            }
        }
        if before == self.her_seen() {
            return false;
        }
        if self.her_visible() {
            self.touch(now);
        } else if self.shape == Shape::Mini {
            // 聊天小窗开着、她那一轮结束了：小窗里的对话照样留着，从这时起再走一圈
            self.touch(now);
        }
        self.bump();
        self.settle(now);
        true
    }

    /// 插件推送了一行（新的、或者更新）：同一条原地换掉。
    pub fn line(&mut self, mut line: Line, now: u64) -> bool {
        line.updated = now;
        match self.lines.iter_mut().find(|l| l.id == line.id) {
            Some(old) => *old = line,
            None => self.lines.push(line),
        }
        if self.lines.len() > MAX_LINES {
            let extra = self.lines.len() - MAX_LINES;
            self.lines.drain(..extra);
        }
        self.touch(now);
        self.bump();
        self.settle(now);
        true
    }

    /// 弹出聊天小窗（新的卡片，或带 alert 的更新），定位到这一条。
    pub fn card(&mut self, conversation_id: &str, plugin: &str, key: &str, now: u64) -> bool {
        self.mini = Some(Mini {
            conversation_id: conversation_id.into(),
            focus: id(plugin, key),
            key: key.trim().into(),
        });
        self.touch(now);
        self.bump();
        self.settle(now);
        true
    }

    /// 推送原地更新了、没有弹出（不带 alert）：聊天小窗开着时小窗里那张变了，进度条从头走；别的时候不用管。
    pub fn updated(&mut self, now: u64) -> bool {
        if !self.shown || self.shape != Shape::Mini {
            return false;
        }
        self.touch(now);
        self.bump();
        self.settle(now);
        true
    }

    /// 插件说这条推送结束了（`push_done`）：收起它的那一行。聊天里的记录留着。
    pub fn done(&mut self, id: &str, now: u64) -> bool {
        let before = self.lines.len();
        self.lines.retain(|l| l.id != id);
        if before == self.lines.len() {
            return false;
        }
        self.bump();
        self.settle(now);
        true
    }

    /// 界面报上来头像旁边放的是什么（`revision` 是它画的那一版）。换了就从头走一圈（小窗的一圈长一些）。
    pub fn shape(&mut self, mini: bool, revision: u64, now: u64) -> bool {
        let shape = if mini { Shape::Mini } else { Shape::Bubble };
        if !self.shown || revision < self.since_revision || shape == self.shape {
            return false;
        }
        self.shape = shape;
        self.touch(now);
        self.bump();
        self.settle(now);
        true
    }

    /// 鼠标进出悬浮窗：在上面时进度条停住，移开接着走。返回进度条变没变。
    pub fn hover(&mut self, inside: bool, now: u64) -> bool {
        if self.hovering == inside || !self.shown {
            return false;
        }
        self.hovering = inside;
        self.settle(now);
        true
    }

    /// 正拖着（Kotone 自己跟着光标挪）：和鼠标在上面一样停住。
    pub fn hold(&mut self, holding: bool, now: u64) -> bool {
        if self.holding == holding {
            return false;
        }
        self.holding = holding;
        if !self.shown {
            return false;
        }
        self.settle(now);
        true
    }

    /// 都收起：她这一行记下收起时的状态。
    fn clear(&mut self, by_player: bool, now: u64) -> bool {
        let shown = self.shown;
        self.lines.clear();
        self.mini = None;
        self.shape = Shape::Bubble;
        if let Some(her) = self.her {
            if self.her_closed.is_none() {
                self.her_closed = Some(Closed {
                    phase: her.phase,
                    by_player,
                });
            }
        }
        if !shown {
            return false;
        }
        self.bump();
        self.settle(now);
        true
    }

    /// ×、打开了 Kotone（推送都在聊天里了）：现在显示着的都收起，直到下一条推送；她这一行收到她换了状态。
    pub fn close(&mut self, now: u64) -> bool {
        self.clear(true, now)
    }

    /// 到点：进度条走完了整个收起；等插件说结束等太久的那一行不再停着进度条。返回界面要不要变。
    pub fn tick(&mut self, now: u64) -> bool {
        if !self.shown {
            return false;
        }
        let running = self.clock.from.is_some();
        self.settle(now);
        if self.clock.from.is_some() && self.clock.left_at(now) == 0 {
            return self.clear(false, now);
        }
        // 等插件说结束的那一行到了时候，进度条开始走：告诉界面
        running != self.clock.from.is_some()
    }

    /// 下一次要看的时间（进度条走完、等插件说结束的那一行到时候；都没有时 None）。
    pub fn next_deadline(&self) -> Option<u64> {
        if !self.shown {
            return None;
        }
        let ring = self.clock.from.map(|from| from + self.clock.left);
        let waiting = if self.hovering || self.holding {
            None
        } else {
            self.lines
                .iter()
                .filter(|l| l.until_done)
                .map(|l| l.updated + UNTIL_DONE_MAX_MS)
                .min()
        };
        ring.into_iter().chain(waiting).min()
    }

    pub fn view(&self, now: u64) -> View {
        View {
            revision: self.revision,
            shown: self.shown,
            her: self.her_visible(),
            lines: self.lines.clone(),
            mini: self.mini.clone(),
            since: self.since,
            ring: self.shown.then(|| Ring {
                total_ms: self.clock.total,
                left_ms: self.clock.left_at(now),
                running: self.clock.from.is_some(),
                at_ms: now,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kotone_plugin_api::PushTone;

    fn cells(texts: &[(&str, Option<PushTone>)]) -> Vec<PushCell> {
        texts
            .iter()
            .map(|(text, tone)| PushCell {
                text: text.to_string(),
                tone: *tone,
            })
            .collect()
    }

    fn pick(key: &str, best: usize) -> Line {
        let mut row = vec![("可选", Some(PushTone::Ok)); 3];
        row[best] = ("首选", Some(PushTone::Good));
        Line::new("lol", "英雄联盟", key, "c1", cells(&row), true)
    }

    fn reminder(key: &str) -> Line {
        Line::new(
            "lol",
            "英雄联盟",
            key,
            "c1",
            cells(&[("对面中单 6 级了", None)]),
            false,
        )
    }

    fn ring(m: &Model, now: u64) -> Ring {
        m.view(now).ring.expect("显示着")
    }

    /// 一行推送：出来、进度条从满的一圈往下走，6 秒走完整个收起。
    #[test]
    fn a_line_runs_the_ring_and_hides_when_it_completes() {
        let mut m = Model::default();
        assert!(!m.view(0).shown);
        assert!(m.line(reminder("lv6:1"), 1_000));
        let v = m.view(1_000);
        assert!(v.shown);
        assert_eq!(v.since, 1_000);
        assert_eq!(v.lines[0].id, "lol:lv6:1");
        let r = ring(&m, 4_000);
        assert_eq!((r.total_ms, r.left_ms, r.running), (BUBBLE_MS, 3_000, true));
        assert_eq!(m.next_deadline(), Some(1_000 + BUBBLE_MS));
        assert!(!m.tick(6_999));
        assert!(m.tick(7_000));
        let v = m.view(7_000);
        assert!(!v.shown && v.lines.is_empty() && v.ring.is_none());
        assert_eq!(m.next_deadline(), None);
    }

    /// 鼠标停在上面：进度条停住，到点也不收；移开接着走（剩下的，至少 2 秒）。
    #[test]
    fn hovering_pauses_the_ring_and_leaving_continues_it() {
        let mut m = Model::default();
        m.line(reminder("lv6:1"), 0);
        assert!(m.hover(true, 2_000));
        let r = ring(&m, 50_000);
        assert_eq!((r.left_ms, r.running), (BUBBLE_MS - 2_000, false));
        assert_eq!(m.next_deadline(), None);
        assert!(!m.tick(50_000));
        assert!(m.view(50_000).shown);
        // 移开：接着走剩下的 4 秒
        assert!(m.hover(false, 50_000));
        assert_eq!(m.next_deadline(), Some(54_000));
        assert!(!m.tick(53_999));
        assert!(m.tick(54_000));
        assert!(!m.view(54_000).shown);
        // 快走完时停住再移开：至少还剩 2 秒
        m.line(reminder("lv6:2"), 100_000);
        m.hover(true, 105_900);
        m.hover(false, 106_000);
        assert_eq!(m.next_deadline(), Some(106_000 + AFTER_LEAVE_MS));
    }

    /// 她在听 / 在想 / 在说时进度条不走；她说完了从满的一圈走，`bubble.rs` 收起她那一行以后最后那句还留着，走完才收。
    #[test]
    fn the_ring_is_held_while_she_talks_and_runs_once_she_is_done() {
        let mut m = Model::default();
        assert!(m.her(Some(Phase::Listening), true, 0));
        let v = m.view(0);
        assert!(v.shown && v.her);
        assert!(!ring(&m, 0).running);
        m.her(Some(Phase::Thinking), true, 3_000);
        m.her(Some(Phase::Speaking), true, 5_000);
        assert!(!m.tick(60_000), "说着的时候走不完");
        let r = ring(&m, 60_000);
        assert_eq!((r.left_ms, r.running), (BUBBLE_MS, false));
        // 念完了：从这时起走一圈
        assert!(m.her(Some(Phase::Speaking), false, 61_000));
        assert!(ring(&m, 61_000).running);
        // bubble.rs 按字数停留后收起她那一行：这里留着最后那句
        assert!(!m.her(None, false, 64_000));
        assert!(m.view(64_000).her);
        assert!(m.tick(61_000 + BUBBLE_MS));
        assert!(!m.view(67_000).shown);
        // 走完收起的：她再开口就再出来
        m.her(Some(Phase::Speaking), false, 70_000);
        m.her(None, false, 70_500);
        assert!(m.her(Some(Phase::Listening), true, 80_000));
        assert!(m.view(80_000).her);
    }

    /// 她还在听 / 在想时被取消：马上收起（没有她说的话要看）；聊天小窗开着时小窗留着，再走一圈。
    #[test]
    fn a_cancelled_turn_hides_her_line_at_once_but_the_mini_chat_stays() {
        let mut m = Model::default();
        m.her(Some(Phase::Listening), true, 0);
        assert!(m.her(None, false, 1_000));
        assert!(!m.view(1_000).shown);
        // 聊天小窗开着（卡片推送）时她开口又被取消
        m.card("c1", "lol", "opening:1", 10_000);
        assert!(m.shape(true, m.view(10_000).revision, 10_100));
        m.her(Some(Phase::Thinking), true, 12_000);
        assert!(!ring(&m, 20_000).running);
        assert!(m.her(None, false, 20_000));
        let v = m.view(20_000);
        assert!(v.shown && !v.her);
        let r = ring(&m, 20_000);
        assert_eq!((r.total_ms, r.left_ms, r.running), (MINI_MS, MINI_MS, true));
        assert!(m.tick(20_000 + MINI_MS));
    }

    /// 界面换成聊天小窗：一圈变成 10 秒、从头走；更早那一版报的不算；小窗开着时推送原地更新也从头走。
    #[test]
    fn the_mini_chat_has_a_longer_ring_and_restarts_on_updates() {
        let mut m = Model::default();
        m.line(reminder("lv6:1"), 0);
        let first = m.view(0).revision;
        m.her(Some(Phase::Listening), true, 1_000);
        // 上一次显示时那一版报的“小窗”不算
        assert!(!m.shape(true, first - 1, 1_000));
        assert!(!m.updated(1_000), "气泡时原地更新不用管");
        m.her(Some(Phase::Speaking), false, 2_000);
        assert!(m.shape(true, m.view(2_000).revision, 2_000));
        assert!(!m.shape(true, m.view(2_000).revision, 2_100), "没变");
        assert_eq!(ring(&m, 2_000).total_ms, MINI_MS);
        assert!(m.updated(8_000));
        assert_eq!(m.next_deadline(), Some(8_000 + MINI_MS));
        assert!(!m.tick(17_999));
        assert!(m.tick(18_000));
        assert!(!m.view(18_000).shown);
        // 收起以后又是气泡
        m.line(reminder("lv6:2"), 30_000);
        assert_eq!(ring(&m, 30_000).total_ms, BUBBLE_MS);
    }

    /// 三选一（until_done）：插件说结束前进度条停着，刷新了原地换掉；插件说结束了接着走；一直不说，最后一次更新后 90 秒走。
    #[test]
    fn an_until_done_line_holds_the_ring_until_the_plugin_says_done() {
        let mut m = Model::default();
        m.line(pick("augment:1", 1), 0);
        assert!(!m.tick(30_000));
        assert!(!ring(&m, 30_000).running);
        m.line(pick("augment:1", 0), 40_000);
        let v = m.view(40_000);
        assert_eq!(v.lines.len(), 1);
        assert_eq!(v.lines[0].cells[0].text, "首选");
        assert!(m.done("lol:augment:1", 41_000));
        assert!(!m.view(41_000).shown, "只有这一行：收起");
        assert!(!m.done("lol:augment:1", 41_000));
        // 和一句提醒一起：插件说结束了，接着走
        m.line(pick("augment:2", 2), 50_000);
        m.line(reminder("lv6:1"), 51_000);
        assert!(m.done("lol:augment:2", 60_000));
        assert!(ring(&m, 60_000).running);
        // 一直不说：最后一次更新后 90 秒开始走
        m.line(pick("augment:3", 2), 200_000);
        assert_eq!(m.next_deadline(), Some(200_000 + UNTIL_DONE_MAX_MS));
        assert!(m.tick(200_000 + UNTIL_DONE_MAX_MS));
        assert!(ring(&m, 200_000 + UNTIL_DONE_MAX_MS).running);
    }

    /// 带 alert 的更新把小窗再弹一次、定位到那一条；× 以后什么都不显示，下一条推送再出来；打开 Kotone 一样收起。
    #[test]
    fn an_alert_pops_again_and_close_waits_for_the_next_push() {
        let mut m = Model::default();
        m.card("c1", "lol", "champ-select:1", 0);
        m.card("c1", "lol", "opening:1", 1_000);
        assert!(m.close(1_500));
        assert!(!m.view(1_500).shown);
        assert!(!m.close(1_600), "没在显示就不用再变");
        m.card("c1", "lol", "champ-select:1", 5_000);
        let v = m.view(5_000);
        assert!(v.shown);
        assert_eq!(v.mini.unwrap().focus, "lol:champ-select:1");
        assert_eq!(v.since, 5_000);
        assert!(m.close(6_000));
        assert!(!m.view(6_000).shown);
    }

    /// × 收起她这一行：她换了状态才再出来（说完了不算）；她说完、那边收起以后再开口照常出来。
    #[test]
    fn close_hides_her_line_until_she_moves_on() {
        let mut m = Model::default();
        m.her(Some(Phase::Speaking), true, 0);
        assert!(m.close(1_000));
        assert!(
            !m.her(Some(Phase::Speaking), true, 2_000),
            "还在说这一段：照旧收着"
        );
        assert!(
            !m.her(Some(Phase::Speaking), false, 3_000),
            "说完了：照旧收着"
        );
        assert!(!m.her(None, false, 5_000), "那边收起了，不再拿出来");
        assert!(!m.view(5_000).shown);
        assert!(m.her(Some(Phase::Speaking), true, 9_000), "下一段：出来");
        m.close(10_000);
        assert!(m.her(Some(Phase::Asking), true, 11_000), "换了状态：出来");
    }

    /// 拖着时不走；最多三行；× 以后下一条推送照样出来。
    #[test]
    fn dragging_holds_and_lines_are_capped() {
        let mut m = Model::default();
        for n in 0..5 {
            m.line(reminder(&format!("r:{n}")), n);
        }
        let v = m.view(5);
        assert_eq!(v.lines.len(), MAX_LINES);
        assert_eq!(v.lines[0].id, "lol:r:2");
        assert!(m.hold(true, 100));
        assert!(!m.tick(100_000));
        assert!(m.hold(false, 100_000));
        assert_eq!(m.next_deadline(), Some(100_000 + BUBBLE_MS - 96));
        m.close(100_001);
        assert!(m.line(reminder("r:9"), 200_000));
        assert!(m.view(200_000).shown);
    }
}
