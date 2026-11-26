//! 悬浮窗现在显示什么（纯状态，不碰窗口；时间用毫秒传进来，便于测试）。ADR 043 D6。
//!
//! 两种状态（[`Mode`]）：
//! - **气泡**：一行字——她在听、在想、在说（`bubble.rs` 的状态机管她这一行什么时候出来、什么时候收），和插件推送的一行
//!   （几格带好坏的字，三选一是“首选 / 可选 / 可刷新”）。
//! - **小窗**：这段对话里的推送（和聊天页同一种卡片，界面画）；新卡片推来（开着“推送时弹出小窗”）、带 `alert` 的更新时弹出，
//!   定位到那一条。
//!
//! 什么时候收（照消息提示和 dsh-orb-cordis 的 `scheduleCollapse`：鼠标在上面时不收，移开后再倒计时）：
//! - 一句提醒的一行 [`LINE_MS`]；清单里写了 `until_done` 的一行（三选一）一直显示到插件说结束（`push_done`），
//!   插件一直不说就在最后一次更新后 [`UNTIL_DONE_MAX_MS`] 收起；
//! - 小窗 [`MINI_MS`]；
//! - 鼠标在上面（或正拖着）时都不收；移开以后至少再留 [`AFTER_LEAVE_MS`]；
//! - ×：现在显示着的都收起，直到下一条推送；她这一行收起到她换了状态（比如说完了回到“通话中”）。

use kotone_plugin_api::PushCell;
use serde::Serialize;

use crate::bubble::Phase;

/// 一句提醒的一行显示多久。
pub const LINE_MS: u64 = 6_000;
/// 小窗显示多久。
pub const MINI_MS: u64 = 8_000;
/// 鼠标移开以后至少再留多久。
pub const AFTER_LEAVE_MS: u64 = 3_000;
/// 写了 `until_done` 的一行，插件一直不说结束时最多留多久（从最后一次更新算）。
pub const UNTIL_DONE_MAX_MS: u64 = 90_000;
/// 气泡里最多几行推送（再多最旧的先走）。
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
    #[serde(skip)]
    until: u64,
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
            until: 0,
        }
    }
}

/// 推送的标识（和聊天页一样：插件 + key）。
pub fn id(plugin: &str, key: &str) -> String {
    format!("{plugin}:{}", key.trim())
}

/// 弹出的小窗：哪段对话、定位到哪一条。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mini {
    pub conversation_id: String,
    /// 定位到的那一条（`插件:key`）：小窗的高度跟着它。
    pub focus: String,
    pub key: String,
    #[serde(skip)]
    until: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Hidden,
    /// 气泡：她这一行和推送的几行。
    Bubble,
    /// 小窗：推送的卡片（她这一行显示着时也在）。
    Mini,
}

/// 给界面的样子。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub revision: u64,
    pub mode: Mode,
    /// 她这一行显示不显示（内容是 `kotone://bubble` 的快照）。
    pub her: bool,
    /// 推送的几行（新的在后面）。小窗里它们已经是推送记录了，界面只在气泡状态画它们。
    pub lines: Vec<Line>,
    pub mini: Option<Mini>,
}

#[derive(Debug, Default)]
pub struct Model {
    revision: u64,
    /// 她这一行现在的状态（没在显示 = None）。
    her: Option<Phase>,
    /// 点 × 时她在哪个状态：换了状态才再出来。
    her_closed: Option<Phase>,
    lines: Vec<Line>,
    mini: Option<Mini>,
    hovering: bool,
    holding: bool,
}

impl Model {
    fn bump(&mut self) -> bool {
        self.revision += 1;
        true
    }

    /// 她这一行（`bubble.rs` 的状态；`None` = 收起了）。返回界面要不要变。
    pub fn her(&mut self, phase: Option<Phase>) -> bool {
        let before = self.her_shown();
        if self.her_closed.is_some() && phase != self.her_closed {
            self.her_closed = None;
        }
        self.her = phase;
        before != self.her_shown() && self.bump()
    }

    fn her_shown(&self) -> bool {
        self.her.is_some() && self.her_closed.is_none()
    }

    /// 插件推送了一行（新的、或者更新）：气泡里显示，同一条原地换掉；小窗开着时它多留一会儿（这一行在小窗里是一条记录）。
    pub fn line(&mut self, mut line: Line, now: u64) -> bool {
        line.until = now
            + if line.until_done {
                UNTIL_DONE_MAX_MS
            } else {
                LINE_MS
            };
        match self.lines.iter_mut().find(|l| l.id == line.id) {
            Some(old) => *old = line,
            None => self.lines.push(line),
        }
        if self.lines.len() > MAX_LINES {
            let extra = self.lines.len() - MAX_LINES;
            self.lines.drain(..extra);
        }
        if let Some(mini) = &mut self.mini {
            mini.until = mini.until.max(now + LINE_MS);
        }
        self.bump()
    }

    /// 弹出小窗（新的卡片，或带 alert 的更新），定位到这一条。
    pub fn card(&mut self, conversation_id: &str, plugin: &str, key: &str, now: u64) -> bool {
        self.mini = Some(Mini {
            conversation_id: conversation_id.into(),
            focus: id(plugin, key),
            key: key.trim().into(),
            until: now + MINI_MS,
        });
        self.bump()
    }

    /// 插件说这条推送结束了（三选一的卡片从屏幕上消失）：收起它的那一行。小窗、聊天里的记录留着。
    pub fn done(&mut self, id: &str) -> bool {
        let before = self.lines.len();
        self.lines.retain(|l| l.id != id);
        before != self.lines.len() && self.bump()
    }

    /// 鼠标进出悬浮窗。在上面时都不收；移开以后至少再留 [`AFTER_LEAVE_MS`]。
    pub fn hover(&mut self, inside: bool, now: u64) {
        if self.hovering == inside {
            return;
        }
        self.hovering = inside;
        if !inside {
            self.left(now);
        }
    }

    /// 正拖着（Kotone 自己跟着光标挪）：和鼠标在上面一样不收。
    pub fn hold(&mut self, holding: bool, now: u64) {
        if self.holding == holding {
            return;
        }
        self.holding = holding;
        if !holding && !self.hovering {
            self.left(now);
        }
    }

    fn left(&mut self, now: u64) {
        let at_least = now + AFTER_LEAVE_MS;
        for line in &mut self.lines {
            line.until = line.until.max(at_least);
        }
        if let Some(mini) = &mut self.mini {
            mini.until = mini.until.max(at_least);
        }
    }

    /// ×：现在显示着的都收起，直到下一条推送；她这一行收起到她换了状态。
    pub fn close(&mut self) -> bool {
        let shown = self.view().mode != Mode::Hidden;
        self.lines.clear();
        self.mini = None;
        if self.her.is_some() {
            self.her_closed = self.her;
        }
        shown && self.bump()
    }

    /// 玩家打开了 Kotone（点了卡片上面那一行、“打开 Kotone”）：推送都在聊天里了，小窗和几行收起；她这一行照旧。
    pub fn opened(&mut self) -> bool {
        let shown = self.mini.is_some() || !self.lines.is_empty();
        self.lines.clear();
        self.mini = None;
        shown && self.bump()
    }

    /// 到点的收起。返回界面要不要变。
    pub fn tick(&mut self, now: u64) -> bool {
        if self.hovering || self.holding {
            return false;
        }
        let before = (self.lines.len(), self.mini.is_some());
        self.lines.retain(|l| l.until > now);
        if self.mini.as_ref().is_some_and(|m| m.until <= now) {
            self.mini = None;
        }
        before != (self.lines.len(), self.mini.is_some()) && self.bump()
    }

    /// 下一次要看的时间（没有要收的、或鼠标在上面时 None）。
    pub fn next_deadline(&self) -> Option<u64> {
        if self.hovering || self.holding {
            return None;
        }
        self.lines
            .iter()
            .map(|l| l.until)
            .chain(self.mini.as_ref().map(|m| m.until))
            .min()
    }

    pub fn view(&self) -> View {
        let her = self.her_shown();
        let mode = if self.mini.is_some() {
            Mode::Mini
        } else if her || !self.lines.is_empty() {
            Mode::Bubble
        } else {
            Mode::Hidden
        };
        View {
            revision: self.revision,
            mode,
            her,
            lines: self.lines.clone(),
            mini: self.mini.clone(),
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

    /// 一行 → 卡片弹小窗 → 鼠标停在上面不收 → 移开后再留一会儿 → 收起。
    #[test]
    fn a_line_then_a_card_pops_the_mini_window_which_waits_for_the_mouse() {
        let mut m = Model::default();
        assert_eq!(m.view().mode, Mode::Hidden);
        assert!(m.line(reminder("lv6:1"), 0));
        let v = m.view();
        assert_eq!(v.mode, Mode::Bubble);
        assert_eq!(v.lines[0].id, "lol:lv6:1");
        assert_eq!(m.next_deadline(), Some(LINE_MS));

        assert!(m.card("c1", "lol", "opening:1", 1_000));
        let v = m.view();
        assert_eq!(v.mode, Mode::Mini);
        assert_eq!(v.mini.as_ref().unwrap().focus, "lol:opening:1");
        // 一行先到点（小窗里它是一条记录），小窗还在
        assert!(m.tick(LINE_MS));
        assert_eq!(m.view().mode, Mode::Mini);
        assert!(m.view().lines.is_empty());

        // 鼠标停在上面：到点也不收
        m.hover(true, 7_000);
        assert_eq!(m.next_deadline(), None);
        assert!(!m.tick(60_000));
        assert_eq!(m.view().mode, Mode::Mini);
        // 移开：至少再留 3 秒
        m.hover(false, 60_000);
        assert_eq!(m.next_deadline(), Some(60_000 + AFTER_LEAVE_MS));
        assert!(!m.tick(62_000));
        assert!(m.tick(63_000));
        assert_eq!(m.view().mode, Mode::Hidden);
    }

    /// 三选一（until_done）：一直显示，刷新了原地换掉；插件说结束才收；插件一直不说，最后一次更新后 90 秒收。
    #[test]
    fn an_until_done_line_stays_until_the_plugin_says_done() {
        let mut m = Model::default();
        m.line(pick("augment:1", 1), 0);
        assert!(!m.tick(30_000), "6 秒、30 秒都还在");
        assert_eq!(m.view().lines.len(), 1);
        // 刷新：同一个 key，原地换掉那一格
        m.line(pick("augment:1", 0), 40_000);
        let v = m.view();
        assert_eq!(v.lines.len(), 1);
        assert_eq!(v.lines[0].cells[0].text, "首选");
        // 同时来一句提醒：两行，提醒先走
        m.line(reminder("lv6:1"), 41_000);
        assert_eq!(m.view().lines.len(), 2);
        assert!(m.tick(41_000 + LINE_MS));
        assert_eq!(m.view().lines[0].id, "lol:augment:1");
        // 插件说结束
        assert!(m.done("lol:augment:1"));
        assert_eq!(m.view().mode, Mode::Hidden);
        assert!(!m.done("lol:augment:1"));
        // 一直不说：最后一次更新后 90 秒
        m.line(pick("augment:2", 2), 200_000);
        assert!(!m.tick(200_000 + UNTIL_DONE_MAX_MS - 1));
        assert!(m.tick(200_000 + UNTIL_DONE_MAX_MS));
    }

    /// 带 alert 的更新把小窗再弹一次、定位到那一条；× 以后什么都不显示，下一条推送再出来。
    #[test]
    fn an_alert_pops_again_and_close_waits_for_the_next_push() {
        let mut m = Model::default();
        m.card("c1", "lol", "champ-select:1", 0);
        m.card("c1", "lol", "opening:1", 1_000);
        assert!(m.close());
        assert_eq!(m.view().mode, Mode::Hidden);
        assert!(!m.close(), "没在显示就不用再变");
        // 选人卡带 alert 更新：再弹，定位到它
        m.card("c1", "lol", "champ-select:1", 5_000);
        let v = m.view();
        assert_eq!(v.mode, Mode::Mini);
        assert_eq!(v.mini.unwrap().focus, "lol:champ-select:1");
        assert_eq!(m.next_deadline(), Some(5_000 + MINI_MS));
        // 打开 Kotone：小窗收起
        assert!(m.opened());
        assert_eq!(m.view().mode, Mode::Hidden);
    }

    /// 她这一行：跟着她的状态出来、收起；× 收起到她换了状态；小窗开着时也显示。
    #[test]
    fn her_line_follows_her_and_close_hides_it_until_she_moves_on() {
        let mut m = Model::default();
        assert!(m.her(Some(Phase::Listening)));
        assert_eq!(m.view().mode, Mode::Bubble);
        assert!(m.view().her);
        assert!(!m.her(Some(Phase::Listening)), "同一个状态再来一次不用变");
        assert!(m.close());
        assert!(!m.view().her);
        assert!(!m.her(Some(Phase::Listening)), "还在听：照旧收着");
        assert!(m.her(Some(Phase::Speaking)), "开口说了：再出来");
        m.card("c1", "lol", "opening:1", 0);
        let v = m.view();
        assert_eq!((v.mode, v.her), (Mode::Mini, true));
        assert!(m.her(None));
        assert!(!m.view().her);
        // 她说完收起以后，× 记的状态也清掉
        m.close();
        m.her(Some(Phase::Speaking));
        m.close();
        m.her(None);
        assert!(m.her(Some(Phase::Speaking)));
    }

    /// 拖着时不收；一行最多三行；新的一条推送 × 以后照样出来。
    #[test]
    fn dragging_holds_and_lines_are_capped() {
        let mut m = Model::default();
        for n in 0..5 {
            m.line(reminder(&format!("r:{n}")), n);
        }
        let v = m.view();
        assert_eq!(v.lines.len(), MAX_LINES);
        assert_eq!(v.lines[0].id, "lol:r:2");
        m.hold(true, 100);
        assert!(!m.tick(100_000));
        m.hold(false, 100_000);
        assert!(m.tick(100_000 + AFTER_LEAVE_MS));
        assert_eq!(m.view().mode, Mode::Hidden);
        m.line(reminder("r:9"), 200_000);
        m.close();
        assert!(m.line(reminder("r:10"), 200_001));
        assert_eq!(m.view().mode, Mode::Bubble);
    }
}
