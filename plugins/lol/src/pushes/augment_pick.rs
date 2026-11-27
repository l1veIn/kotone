//! 海克斯三选一 / 四选一（ADR 043 D7，2026-11-26 晚修订：推一张卡片，识别范围收窄；2026-11-27 修订：认四选一，语音说
//! “选第几个”）：清单 `[[pushes]]` 的 `augment-pick`。
//!
//! - **看画面**：海斗对局里只截卡片**名字所在的那一横条**（[`NAME_STRIP`]，照 AramHex 的标题位置，见下）识别文字
//!   （宿主的 `screen_text`，**固定用本地文字识别**，和全局的识别设置无关），认出海克斯的名字（aramhex-data 的海克斯名），
//!   再看这些名字是不是**同一行、以画面中线对称、等距排开**，排得像三张还是四张（[`offer_of`]，规矩见下）：至少认出两个，
//!   没认出的那几格在该在的地方同一行有字，才算出现了三选一 / 四选一；别的（左下角聊天框里队友打的海克斯名、卡片说明里
//!   提到的名字）一律不算。按这局自己英雄的 英雄 × 海克斯 胜率、照 AramHex 的规则（[`rules::evaluate`]）给每张一个判断。
//!   卡片在屏幕上时每秒看一次（本地识别一次 0.1–0.3 秒），不在时每 2 秒。
//! - **一张卡片 + 语音**（玩家：“不需要单独给海克斯做一个UI，就直接推卡片就行啊，就像战绩一样推送，卡片上写清楚三个海克斯
//!   推荐程度就行”）：卡片（页面 `ui://kotone/lol/augment-pick`）从左到右三格或四格（和游戏里一样）：图标、名字、评级、
//!   首选 / 可选 / 可刷新 / 没数据（认不出的那张写“没认出”），来源标签 aramhex-data。一次三选一一个 key：刷新了哪张卡片
//!   就原地换哪一格（同一个 key 再推，宿主原地更新；三张、四张之间变了也是同一次里变了）；她在卡片刚出现时说一次
//!   （玩家 2026-11-27：“语音也需要改成，选第一个、选第二个、选第三个、选第四个”——三张时也按第几个说，几个说法轮着用），
//!   刷新后最好的那张变了才再说（说法跟着结论走：结论没变，说法就没变，宿主不再说）。都不好时说“都一般，可以刷新”。
//!   同样的画面连着看到两次才推（刷新的动画里只认出一部分时不推，免得说错）。卡片没了 [`CARD_GONE`]：卡片标上“选好了”，
//!   `push_done`（聊天里的记录留着）；之后再出三选一是新的一次（新的 key）。
//! - **本地识别用不了时**：一个本地模型都没装、玩家也没说过“不用”——推一张卡片问要不要下载（页面
//!   `ui://kotone/lol/augment-ocr`，下载 / 不用；海斗选英雄时就问，进了游戏点不了）；下载走本体的模型下载，不改全局的
//!   文字识别选择。用不了（没有模型又说了不用、下载失败、游戏是独占全屏、识别出错）时推一行说清楚原因，**每局最多一次**；
//!   游戏不在前台不算用不了（不截，等它回到前台）。
//!
//! **识别的位置和怎么认几张**照 [AramHex](https://github.com/czgreat/aramhex)（MIT，Copyright (c) 2026 czgreat；读的是
//! `main` 分支 `ffb784a`，2026-11-26），许可证全文在 `licenses/aramhex-MIT.txt`：
//! - 标题在哪一行：`lolhex/vision/detector.py` 的 `DEFAULT_BOXES` 在窗口高度的 38.7%–43.4%（国服 4K 实机校准，游戏界面
//!   按高度缩放、水平居中）。判定“不靠固定坐标，看画面里是否同时出现 ≥2 个能匹配到海克斯字典的文字，且它们在同一水平线上、
//!   左右排开”（`_pick_row`：同一行是纵向相差 0.03 个窗口高度以内，这里收紧到 [`ROW_TOLERANCE`]）。
//! - 四选一（它 1.1.7 起认，`docs/recognition-settings.md`「四选一」、`_layout_columns`、`tests/test_four_choice.py`）：
//!   “四张卡的卡距、卡宽和标题高度与三选一相同，整组仍以画面中线对称，只是错开半张：三选一卡心在中线和中线 ±1 个卡距，
//!   四选一在 ±0.5、±1.5 个卡距”；卡距约 0.342 个窗口高度（`CARD_SPACING_H`，它的四选一实机截图也是约 0.34）。它 4K 实录的
//!   四选一标题框左边在 13.1%、32.3%、51.5%、70.7% 屏宽（卡心约 21%、40%、60%、79%）。它记的是“测试服新地图”；英雄联盟 wiki
//!   的 ARAM: Mayhem 页（2026-11-27 读）写着每局开场有一个 Headliner，其中 Maestro 是“Augment selection screens now contain
//!   four offerings of augments, rather than three.”——玩家说的“有时候会是四选一”应该就是它。
//! - 和它不一样：它知道窗口多大（卡距按窗口高度算，两张时用实机卡距推），还有整块扫描 + 本机校准；这里只截固定的一横条，
//!   拿到的是客户区的比例坐标、不知道宽高比，所以卡距由认出来的名字自己推（整组以中线对称，最小二乘），只认合理的卡距
//!   （[`SPACING`]，5:4 到 32:9）；没认出的那一格要它的卡心附近同一行有字（[`NEAR`]）——这样只认出两张时也分得清三张、四张
//!   （例如左右对称的两个名字：三选一的中间那格在中线上，四选一中间两格在 ±1/3 的位置，只有一种有字）。
//! - **没在真机上校准过**：四选一的位置只来自 AramHex 的说明、测试和实录数字，我们自己没截过四选一的画面；横条左右放宽到
//!   2%–98% 是按卡距算的（4:3 的四选一最外侧卡心在 11.5% / 88.5% 屏宽），没在 16:10、4:3 的屏幕上看过。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kotone_core::log;
use kotone_plugin_api::{
    Host, LocalOcr, Push, PushCard, PushCell, PushLine, PushOutcome, PushTone, ScreenText,
    ToolResult,
};
use serde_json::{json, Value};

use super::rules::{self, CardVerdict, Verdict};
use super::{now_ms, PushState, PAGE_AUGMENT_OCR, PAGE_AUGMENT_PICK, PUSH_AUGMENT_PICK};
use crate::client::GameData;
use crate::stats::aramhex;
use crate::LolPlugin;

const GAME_EXE: &str = "League of Legends.exe";
/// 卡片名字所在的那一横条（游戏窗口客户区的比例 `[x, y, w, h]`）：只截这一条。纵向 34%–48%：AramHex 的标题框在
/// 38.7%–43.4%（三张、四张一样高），上下各留约 4.5 个点（不同电脑上差一点也盖得住）；左下角的聊天框在 55% 以下，截不到。
/// 横向 2%–98%：三选一 16:9 的名字中点约在 31%、50%、70%；四选一最外侧的卡心离中线 1.5 个卡距（约 0.51 个窗口高度），
/// 16:9 在 21% / 79%，4:3 在 11.5% / 88.5%，再留出长名字的一半（照 AramHex `_band` 给窄屏四选一放宽；原来的 12%–88%
/// 盖不住 16:10 及更窄的四选一）。
pub(crate) const NAME_STRIP: [f64; 4] = [0.02, 0.34, 0.96, 0.14];
/// 同一行：纵向中点相差不到这么多（窗口高度的比例；AramHex 是 0.03，这里收紧一点，卡片说明的第一行离标题约 0.03）。
const ROW_TOLERANCE: f64 = 0.02;
/// 三选一、四选一的卡心（以画面中线为 0、卡距为 1；AramHex `_layout_columns`）：四选一错开半张。
const LAYOUTS: [&[f64]; 2] = [&[-1.0, 0.0, 1.0], &[-1.5, -0.5, 0.5, 1.5]];
/// 卡距（画面宽度的比例）的合理范围：卡距约 0.342 个窗口高度，5:4 约 0.27、16:9 约 0.19、21:9 约 0.15、32:9 约 0.096。
const SPACING: std::ops::RangeInclusive<f64> = 0.08..=0.30;
/// 认出来的名字离它那一格的卡心最多差几个卡距（AramHex 判四选一用 0.25，这里紧一点）。
const FIT: f64 = 0.2;
/// 没认出的那一格：同一行、离它的卡心不到这么多个卡距的地方有字才算。要比 1/3 小：三选一左右两张认出来时中间那格在中线上，
/// 四选一最外两张认出来时中间两格在 ±1/3（按三选一的卡距算）的地方，靠这个分开。
const NEAR: f64 = 0.25;
/// 横向中点差不到这么多（画面宽度的比例）算同一张卡片上的（AramHex 是 0.08 个窗口高度，16:9 约 0.045）。
const SAME_CARD: f64 = 0.04;
/// 卡片的名字（三选一 / 四选一）。
pub(crate) const PICK_CARD_TITLE: &str = "海克斯三选一";
pub(crate) const PICK_CARD_TITLE_FOUR: &str = "海克斯四选一";
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
/// 第几个（她说的话里的位置，从左数；三选一、四选一一样说）。
const ORDINALS: [&str; 4] = ["第一个", "第二个", "第三个", "第四个"];

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

fn center_x(line: &ScreenText) -> f64 {
    line.rect[0] + line.rect[2] / 2.0
}

fn center_y(line: &ScreenText) -> f64 {
    line.rect[1] + line.rect[3] / 2.0
}

/// 认出来的名字放进格子的一种排法。
#[derive(Debug, Clone, PartialEq)]
struct Layout {
    /// 从左到右每格是哪个海克斯（None = 这一格有字但没认出）：3 或 4 格。
    slots: Vec<Option<i64>>,
    /// 认出来的名字离它那格的卡心最远差几个卡距（越小排得越齐）。
    err: f64,
}

/// `n` 格里挑 `k` 格（从左到右）的所有挑法。
fn choose(n: usize, k: usize) -> Vec<Vec<usize>> {
    (0u32..1 << n)
        .filter(|mask| mask.count_ones() as usize == k)
        .map(|mask| (0..n).filter(|i| mask & (1 << i) != 0).collect())
        .collect()
}

/// 同一行（纵向中点 `y`）认出来的名字（横向中点、海克斯，从左到右）能怎么排进三格 / 四格：每个名字放第几格（从左到右、
/// 不重格）都试一遍，卡距用最小二乘推（整组以画面中线对称），卡距在 [`SPACING`] 里、每个名字离它那格的卡心不到 [`FIT`]
/// 个卡距才算；剩下的格子要卡心附近（[`NEAR`] 个卡距以内）同一行有至少 2 个字。排得最齐的那种；一样齐时三张优先
/// （AramHex：拿不准时按三选一）。
fn layout_of(found: &[(f64, i64)], y: f64, lines: &[ScreenText]) -> Option<Layout> {
    let mut best: Option<Layout> = None;
    for offsets in LAYOUTS {
        for at in choose(offsets.len(), found.len()) {
            let o: Vec<f64> = at.iter().map(|&i| offsets[i]).collect();
            let u: Vec<f64> = found.iter().map(|(x, _)| x - 0.5).collect();
            let oo: f64 = o.iter().map(|v| v * v).sum();
            let spacing = o.iter().zip(&u).map(|(a, b)| a * b).sum::<f64>() / oo;
            if !SPACING.contains(&spacing) {
                continue;
            }
            let err = o
                .iter()
                .zip(&u)
                .map(|(a, b)| (b - a * spacing).abs() / spacing)
                .fold(0.0, f64::max);
            if err > FIT || best.as_ref().is_some_and(|b| b.err <= err) {
                continue;
            }
            let mut slots: Vec<Option<i64>> = vec![None; offsets.len()];
            for (&i, (_, id)) in at.iter().zip(found) {
                slots[i] = Some(*id);
            }
            let filled = offsets.iter().enumerate().all(|(i, offset)| {
                slots[i].is_some()
                    || lines.iter().any(|line| {
                        (center_y(line) - y).abs() <= ROW_TOLERANCE
                            && (center_x(line) - (0.5 + offset * spacing)).abs() <= NEAR * spacing
                            && norm(&line.text).chars().count() >= 2
                    })
            });
            if filled {
                best = Some(Layout { slots, err });
            }
        }
    }
    best
}

/// 画面上有没有三选一 / 四选一：从左到右每格是哪个海克斯（`None` = 那一格有字但没认出），一共 3 或 4 格。
///
/// 规矩（照 AramHex 的“同一水平线上、左右排开”和三张 / 四张的排法，见文件头）：认出来的名字里挑一行（纵向中点相差不到
/// [`ROW_TOLERANCE`]），同一张卡片上的（横向差不到 [`SAME_CARD`]）只留一个；至少认出两个、按 [`layout_of`] 排得进三格或
/// 四格才算——认出来的名字以画面中线对称、等距（卡距合理），没认出的格子该在的地方同一行有字（新海克斯、名字被挡住）。
/// 挤在一处的（聊天框里队友打的名字、一行里写了好几个名字）、不在同一行的、排不齐的、只认出一个的，都不算。这一行里多认出
/// 一个排不进去的（别处的字碰巧对上了名字）时，去掉它再排。
pub fn offer_of(lines: &[ScreenText], d: &aramhex::Dataset) -> Option<Vec<Option<i64>>> {
    let hits = match_augments(lines, d);
    let mut best: Option<(usize, Layout)> = None;
    for (anchor, _) in &hits {
        let anchor_y = center_y(anchor);
        // 这一行：(横向中点, 纵向中点, 离这一行多远, 海克斯)，同一张卡片上的留离得近的那个
        let mut row: Vec<(f64, f64, f64, i64)> = Vec::new();
        for (line, id) in &hits {
            let dy = (center_y(line) - anchor_y).abs();
            if dy > ROW_TOLERANCE {
                continue;
            }
            let x = center_x(line);
            match row.iter_mut().find(|r| (r.0 - x).abs() < SAME_CARD) {
                Some(kept) if dy < kept.2 => *kept = (x, center_y(line), dy, *id),
                Some(_) => {}
                None => row.push((x, center_y(line), dy, *id)),
            }
        }
        row.sort_by(|a, b| a.0.total_cmp(&b.0));
        row.truncate(6);
        // 认出的越多越好：先拿整行排，排不进再去掉一个、两个……（至少两个）
        for k in (2..=row.len().min(4)).rev() {
            if best.as_ref().is_some_and(|(count, _)| *count >= k) {
                break;
            }
            let found = choose(row.len(), k).into_iter().find_map(|pick| {
                let picked: Vec<_> = pick.iter().map(|&i| row[i]).collect();
                let y = picked.iter().map(|r| r.1).sum::<f64>() / k as f64;
                let found: Vec<(f64, i64)> = picked.iter().map(|r| (r.0, r.3)).collect();
                layout_of(&found, y, lines)
            });
            if let Some(layout) = found {
                best = Some((k, layout));
                break;
            }
        }
    }
    best.map(|(_, layout)| layout.slots)
}

/// 这一次三选一 / 四选一现在的样子：从左到右每格是什么判断（None = 这一格没认出来），每格的详细（卡片上画），首选在
/// 第几格，是不是都不好。三选一 3 格、四选一 4 格。
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub slots: Vec<Option<Verdict>>,
    pub cards: Vec<Option<CardVerdict>>,
    pub best: Option<usize>,
    pub weak: bool,
}

impl View {
    /// 每格认出来的海克斯（[`offer_of`]）和这局自己的英雄。
    pub fn new(offer: &[Option<i64>], d: &aramhex::Dataset, hero: i64) -> Self {
        let ids: Vec<i64> = offer.iter().flatten().copied().collect();
        let judged = rules::evaluate(d, hero, &ids);
        let mut next = judged.iter();
        let cards: Vec<Option<CardVerdict>> = offer
            .iter()
            .map(|id| id.and_then(|_| next.next().cloned()))
            .collect();
        let slots: Vec<Option<Verdict>> = cards
            .iter()
            .map(|c| c.as_ref().map(|c| c.verdict))
            .collect();
        Self {
            best: slots.iter().position(|s| *s == Some(Verdict::Best)),
            weak: rules::all_weak(&judged),
            slots,
            cards,
        }
    }

    /// 卡片的数据（页面 `ui://kotone/lol/augment-pick`）：`count` 一共几格（3 或 4），`slots` 从左到右每格
    /// `{id, name, rarity, icon, grade, verdict, few}`（`few`：样本偏少）或 `{unread: true}`；首选在第几格、是不是都不好；
    /// `ended`：这一次结束了（选好了）。来源标签是 aramhex-data（ADR 043 D8），带数据的版本。
    pub fn card(
        &self,
        d: &aramhex::Dataset,
        data: Option<&GameData>,
        champion: Value,
        ended: bool,
    ) -> Value {
        let slots: Vec<Value> = self
            .cards
            .iter()
            .map(|card| match card {
                Some(c) => {
                    let a = d.augments.get(&c.augment_id);
                    json!({
                        "id": c.augment_id,
                        "name": a.map(|a| a.name.as_str()).unwrap_or(""),
                        "rarity": c.rarity,
                        "icon": data.and_then(|g| g.augments.get(&c.augment_id)).map(|g| g.icon.as_str()).unwrap_or(""),
                        "grade": c.grade,
                        "verdict": c.verdict,
                        "few": c.sample_warning || c.thin.is_some(),
                    })
                }
                None => json!({"unread": true}),
            })
            .collect();
        json!({
            "champion": champion,
            "count": slots.len(),
            "slots": slots,
            "best": self.best,
            "weak": self.weak,
            "source": "aramhex",
            "patch": d.patch,
            "ended": ended,
        })
    }

    /// 每格的判断写成字（卡片上的字由页面照 `verdict` 画；这里给测试）：首选 / 可选 / 可刷新 / 没数据 / 没认出。
    #[cfg(test)]
    pub fn cells(&self) -> Vec<&'static str> {
        self.slots
            .iter()
            .map(|slot| match slot {
                Some(Verdict::Best) => "首选",
                Some(Verdict::Ok) => "可选",
                Some(Verdict::Reroll) => "可刷新",
                Some(Verdict::NoData) => "没数据",
                None => "没认出",
            })
            .collect()
    }

    /// 她说的那句（几个说法，宿主轮着用）：首选是从左数第几个（三张、四张都这样说）；都不好时说可以刷新。一张有数据的
    /// 都没有时不说。说法只跟结论走（首选在第几个、是不是都不好），不提一共几张：刷新以后结论没变，说法就一样，宿主不再说
    /// （三张、四张之间认变了也一样）。
    pub fn voice(&self) -> Vec<String> {
        let Some(best) = self.best else {
            return Vec::new();
        };
        let nth = ORDINALS[best.min(ORDINALS.len() - 1)];
        if self.weak {
            vec![
                "都一般，可以刷新".to_string(),
                format!("这几张都不太好，刷新吧，不然选{nth}"),
            ]
        } else {
            vec![format!("选{nth}"), format!("{nth}最好"), format!("拿{nth}")]
        }
    }
}

/// 画面上认出来的三选一 / 四选一（[`offer_of`] 的规矩）。
pub fn view_of(lines: &[ScreenText], d: &aramhex::Dataset, hero: i64) -> Option<View> {
    offer_of(lines, d).map(|offer| View::new(&offer, d, hero))
}

// ---------- 一帧一帧地看 ----------

/// 这一次看画面看到了什么。
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    /// 看到了三选一。
    Cards(Box<View>),
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
    /// 这一次三选一结束了（卡片没了、这局结束了）：卡片标上“选好了”（最后推出去的样子），`push_done`。
    Done { key: String, last: View },
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
                        let open = self.open.take().expect("上面看过");
                        vec![Step::Done {
                            key: open.key,
                            last: open.shown,
                        }]
                    }
                    _ => Vec::new(),
                }
            }
            Frame::Cards(view) => {
                let view = *view;
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
        self.open.take().map(|open| Step::Done {
            key: open.key,
            last: open.shown,
        })
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
    /// 最后推出去的那张三选一卡片（key、数据、说法）：结束时标上“选好了”再推一次（说法不变，她不会再说）。
    last_card: Option<(String, Value, Vec<String>)>,
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

/// 三选一的卡片（同一个 key 再推就是原地更新）：四格时卡片叫“海克斯四选一”。
fn pick_push(key: String, card: Value, voice: Vec<String>) -> Push {
    let title = if card["count"] == 4 {
        PICK_CARD_TITLE_FOUR
    } else {
        PICK_CARD_TITLE
    };
    Push {
        trigger: PUSH_AUGMENT_PICK.into(),
        key,
        card: Some(PushCard {
            title: title.into(),
            uri: PAGE_AUGMENT_PICK.into(),
            params: Value::Null,
            data: card,
        }),
        voice,
        ..Push::default()
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
        if let Some(Step::Done { key, .. }) = step {
            self.finish_pick(host, &key);
        }
    }

    /// 这一次三选一结束了：卡片标上“选好了”（同一个 key、说法不变），`push_done`。
    fn finish_pick(&self, host: &dyn Host, key: &str) {
        let last = self.pushes.lock().unwrap().pick.last_card.take();
        if let Some((last_key, mut card, voice)) = last.filter(|(k, _, _)| k == key) {
            card["ended"] = json!(true);
            pushed(PICK_CARD_TITLE, host.push(pick_push(last_key, card, voice)));
        }
        host.push_done(PUSH_AUGMENT_PICK, key);
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
            if let Some(Step::Done { key, .. }) = step {
                self.finish_pick(host.as_ref(), &key);
            }
            return IDLE_POLL;
        };
        if !self.local_ocr_ready(host, live.game_id) {
            return IDLE_POLL;
        }
        let mut seen = None;
        let frame = match host.screen_text(GAME_EXE, NAME_STRIP, true).await {
            Ok(lines) => {
                self.pushes.lock().unwrap().pick.black = 0;
                match self.offer_view(host.as_ref(), &lines, &live.alias).await {
                    Some((view, hex, champion)) => {
                        seen = Some((hex, champion));
                        Frame::Cards(Box::new(view))
                    }
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
                    let Some((hex, champion)) = &seen else {
                        continue;
                    };
                    let card =
                        view.card(hex, self.loaded_data().as_deref(), champion.clone(), false);
                    let voice = view.voice();
                    self.pushes.lock().unwrap().pick.last_card =
                        Some((key.clone(), card.clone(), voice.clone()));
                    pushed(PICK_CARD_TITLE, host.push(pick_push(key, card, voice)));
                }
                Step::Done { key, .. } => self.finish_pick(host.as_ref(), &key),
            }
        }
        if self.pushes.lock().unwrap().pick.tracker.is_open() {
            CARD_POLL
        } else {
            IDLE_POLL
        }
    }

    /// 画面上的三选一（这局自己的英雄、aramhex-data 的数据都有时才认得出）：样子、数据和卡片上的英雄 `{id, short}`。
    async fn offer_view(
        &self,
        host: &dyn Host,
        lines: &[ScreenText],
        alias: &str,
    ) -> Option<(View, Arc<aramhex::Dataset>, Value)> {
        let hex = self.stats_for(host).aramhex().await.ok()?;
        let data = self.loaded_data()?;
        let hero = data.champion_by_alias(alias)?;
        let view = view_of(lines, &hex, hero.id)?;
        Some((view, hex, json!({"id": hero.id, "short": hero.short()})))
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

    /// 识别出来的一帧（只截了名字那一横条）：左（中点 x 0.31）、中（0.5）、右（0.7）三张卡片的名字在同一行
    /// （中点 y 0.41，照 AramHex 的标题位置），下面是说明的第一行。
    fn ocr(left: &str, middle: &str, right: &str) -> Vec<ScreenText> {
        vec![
            line(right, 0.65, 0.395),
            line(left, 0.26, 0.395),
            line(middle, 0.45, 0.398),
            line("每次攻击……", 0.45, 0.44),
        ]
    }

    /// 16:9 的四选一（AramHex 4K 实录：标题框左边在 13.1%、32.3%、51.5%、70.7% 屏宽，框宽约 16.1%，卡心约 21.2%、40.4%、
    /// 59.6%、78.8%）：四个名字在同一行，下面是说明的第一行。
    const FOUR: [f64; 4] = [0.2121, 0.4039, 0.5957, 0.7877];
    fn ocr4(names: [&str; 4]) -> Vec<ScreenText> {
        let mut lines: Vec<ScreenText> = names
            .iter()
            .zip(FOUR)
            .map(|(name, cx)| line(name, cx - 0.05, 0.395))
            .collect();
        lines.reverse();
        lines.push(line("每次攻击……", FOUR[1] - 0.05, 0.44));
        lines
    }

    #[test]
    fn three_cards_are_found_and_judged_left_to_right() {
        let d = dataset();
        let lines = ocr("鼓声暴起·低音轰鸣", "弹跳 蜜蜂", "终极九头蛇");
        let found = match_augments(&lines, &d);
        let ids: Vec<i64> = found.iter().map(|(_, id)| *id).collect();
        assert_eq!(ids, vec![2, 4, 3], "从左到右，长名字优先");
        assert_eq!(offer_of(&lines, &d), Some(vec![Some(2), Some(4), Some(3)]));
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
        assert_eq!(view.cells(), ["可选", "可刷新", "首选"]);
        // 玩家 2026-11-27：“选第一个、选第二个、选第三个、选第四个”——三张时也按第几个说，不说左边 / 中间 / 右边
        assert_eq!(view.voice(), ["选第三个", "第三个最好", "拿第三个"]);
        for v in view.voice() {
            assert!(v.chars().count() <= Push::VOICE_MAX, "{v}");
            assert!(!["左", "中间", "右"].iter().any(|w| v.contains(w)), "{v}");
        }
    }

    /// 玩家 2026-11-27：“现在的海斗有时候会是四选一”。四个名字同一行、以中线对称等距排开：四格，从左到右；首选在第几个就说
    /// 第几个。
    #[test]
    fn four_cards_are_found_left_to_right_and_said_by_number() {
        let d = dataset();
        let lines = ocr4(["弹跳蜜蜂", "终极九头蛇", "鼓声暴起 低音轰鸣", "冰川之心"]);
        assert_eq!(
            offer_of(&lines, &d),
            Some(vec![Some(4), Some(3), Some(2), Some(6)])
        );
        let view = view_of(&lines, &d, 39).unwrap();
        // 终极九头蛇 +7.8 首选；鼓声暴起 低音轰鸣 +6.6 低不到 2 个点：可选；弹跳蜜蜂、冰川之心比平均低：可刷新
        assert_eq!(view.cells(), ["可刷新", "首选", "可选", "可刷新"]);
        assert_eq!(view.best, Some(1));
        assert_eq!(view.voice(), ["选第二个", "第二个最好", "拿第二个"]);
        // 首选在最右边：第四个
        let right = view_of(
            &ocr4(["弹跳蜜蜂", "冰川之心", "终极九头蛇", "珠光护手"]),
            &d,
            39,
        )
        .unwrap();
        assert_eq!(right.best, Some(3));
        assert_eq!(right.voice()[0], "选第四个");
        // 四张里有一张没认出（新海克斯、名字被挡住）：哪一张都行，那一格写“没认出”，别的位置不乱
        for missing in 0..4 {
            let mut names = ["弹跳蜜蜂", "终极九头蛇", "鼓声暴起 低音轰鸣", "冰川之心"];
            names[missing] = "？？新的";
            let mut want = vec![Some(4), Some(3), Some(2), Some(6)];
            want[missing] = None;
            assert_eq!(
                offer_of(&ocr4(names), &d),
                Some(want),
                "第 {missing} 格没认出"
            );
        }
        // 只认出两张（最外两张、中间两张、挨着的两张），别的格子有字：照样认得出是四张
        for (names, want) in [
            (
                ["终极九头蛇", "看不清", "糊了", "珠光护手"],
                vec![Some(3), None, None, Some(5)],
            ),
            (
                ["看不清", "终极九头蛇", "珠光护手", "糊了"],
                vec![None, Some(3), Some(5), None],
            ),
            (
                ["终极九头蛇", "珠光护手", "看不清", "糊了"],
                vec![Some(3), Some(5), None, None],
            ),
        ] {
            assert_eq!(offer_of(&ocr4(names), &d), Some(want), "{names:?}");
        }
        // 四张卡片只认出两张、别的格子没字（刷新的动画里）：不算
        let two = vec![
            line("终极九头蛇", FOUR[0] - 0.05, 0.395),
            line("珠光护手", FOUR[3] - 0.05, 0.395),
        ];
        assert_eq!(offer_of(&two, &d), None);
    }

    /// 三张、四张都是以画面中线对称、等距（卡距约 0.342 个窗口高度）：从 5:4 到 32:9 的窗口，三张认成三张、四张认成四张，
    /// 只认出两张时也不弄混（左右对称的两张：三选一中间那格在中线上，四选一中间两格在 ±1/3 的地方）；名字都在横条里。
    #[test]
    fn three_or_four_by_layout_on_every_aspect_ratio() {
        let d = dataset();
        let names = ["终极九头蛇", "珠光护手", "冰川之心", "弹跳蜜蜂"];
        let ids = [3, 5, 6, 4];
        let [x, y, w, h] = NAME_STRIP;
        assert!(y >= 0.30 && y + h <= 0.5 && h <= 0.15, "{NAME_STRIP:?}");
        for aspect in [
            5.0 / 4.0,
            4.0 / 3.0,
            16.0 / 10.0,
            16.0 / 9.0,
            21.0 / 9.0,
            32.0 / 9.0,
        ] {
            let spacing = 0.3416 / aspect;
            for offsets in LAYOUTS {
                let centers: Vec<f64> = offsets.iter().map(|o| 0.5 + o * spacing).collect();
                // 名字（宽约 0.05）整个在横条里
                assert!(
                    centers[0] - 0.025 >= x && centers[centers.len() - 1] + 0.025 <= x + w,
                    "宽高比 {aspect:.2}、{} 张：{centers:?}",
                    centers.len()
                );
                let at = |i: usize, text: &str| ScreenText {
                    text: text.into(),
                    rect: [centers[i] - 0.025, 0.395, 0.05, 0.03],
                };
                let all: Vec<ScreenText> = (0..centers.len()).map(|i| at(i, names[i])).collect();
                let want: Vec<Option<i64>> = (0..centers.len()).map(|i| Some(ids[i])).collect();
                assert_eq!(offer_of(&all, &d), Some(want), "宽高比 {aspect:.2}");
                // 只认出最外两张、别的格子有字但没认出
                let last = centers.len() - 1;
                let mut outer: Vec<ScreenText> = vec![at(0, names[0]), at(last, names[last])];
                outer.extend((1..last).map(|i| at(i, "看不清")));
                let mut want = vec![None; centers.len()];
                want[0] = Some(ids[0]);
                want[last] = Some(ids[last]);
                assert_eq!(
                    offer_of(&outer, &d),
                    Some(want),
                    "宽高比 {aspect:.2}、只认出最外两张"
                );
                // 只认出最左边两张
                let mut left: Vec<ScreenText> = vec![at(0, names[0]), at(1, names[1])];
                left.extend((2..centers.len()).map(|i| at(i, "看不清")));
                let mut want = vec![None; centers.len()];
                want[0] = Some(ids[0]);
                want[1] = Some(ids[1]);
                assert_eq!(
                    offer_of(&left, &d),
                    Some(want),
                    "宽高比 {aspect:.2}、只认出左边两张"
                );
            }
        }
    }

    /// 卡片的数据：从左到右三格（图标、名字、评级、判断），认不出的那格是 `unread`；来源是 aramhex-data。
    /// 不带胜率数字（页面也不画）。
    #[test]
    fn the_card_lists_three_augments_left_to_right() {
        let d = dataset();
        let view = view_of(&ocr("鼓声暴起 低音轰鸣", "？？", "终极九头蛇"), &d, 39).unwrap();
        let mut data = crate::client::gamedata::tests::sample();
        data.augments.insert(
            3,
            crate::client::gamedata::Augment {
                name: "终极九头蛇".into(),
                rarity: "kPrismatic".into(),
                icon: "/lol-game-data/assets/ASSETS/UX/Cherry/Augments/Icons/Hydra_small.png"
                    .into(),
            },
        );
        let card = view.card(
            &d,
            Some(&data),
            json!({"id": 39, "short": "艾瑞莉娅"}),
            false,
        );
        assert_eq!(card["slots"].as_array().unwrap().len(), 3);
        assert_eq!(card["count"], 3);
        assert_eq!(card["slots"][0]["name"], "鼓声暴起 低音轰鸣");
        assert_eq!(card["slots"][0]["verdict"], "ok");
        assert_eq!(card["slots"][0]["rarity"], "prismatic");
        assert_eq!(card["slots"][1], json!({"unread": true}));
        assert_eq!(card["slots"][2]["verdict"], "best");
        assert!(card["slots"][2]["icon"]
            .as_str()
            .unwrap()
            .ends_with("Hydra_small.png"));
        assert!(card["slots"][2]["grade"].is_string());
        assert_eq!(card["best"], 2);
        assert_eq!(card["source"], "aramhex");
        assert_eq!(card["patch"], "16.23");
        assert_eq!(card["ended"], false);
        assert_eq!(card["champion"]["short"], "艾瑞莉娅");
        let text = card.to_string();
        assert!(
            !text.contains("winRate") && !text.contains("delta"),
            "卡片上不放胜率：{text}"
        );
        assert_eq!(
            pick_push("k".into(), card, view.voice())
                .card
                .unwrap()
                .title,
            "海克斯三选一"
        );
        // 四选一：四格（一共几格写在 count 里），卡片叫“海克斯四选一”
        let four = view_of(
            &ocr4(["弹跳蜜蜂", "终极九头蛇", "？？", "冰川之心"]),
            &d,
            39,
        )
        .unwrap();
        let card = four.card(
            &d,
            Some(&data),
            json!({"id": 39, "short": "艾瑞莉娅"}),
            false,
        );
        assert_eq!(card["count"], 4);
        assert_eq!(card["slots"].as_array().unwrap().len(), 4);
        assert_eq!(card["slots"][2], json!({"unread": true}));
        assert_eq!(card["slots"][1]["verdict"], "best");
        assert!(card["slots"][1]["icon"]
            .as_str()
            .unwrap()
            .ends_with("Hydra_small.png"));
        assert_eq!(card["best"], 1);
        assert_eq!(
            pick_push("k".into(), card, four.voice())
                .card
                .unwrap()
                .title,
            "海克斯四选一"
        );
    }

    #[test]
    fn ordinary_text_is_not_a_card_and_a_missing_card_is_said_so() {
        let d = dataset();
        assert!(
            match_augments(&[line("对面拿了火龙", 0.4, 0.3), line("鼓", 0.1, 0.1)], &d).is_empty()
        );
        assert!(
            view_of(&ocr("对面拿了火龙", "终极九头蛇", "看"), &d, 39).is_none(),
            "只认出一张不算"
        );
        // 中间那张认不出（新海克斯、名字被挡住）：那一格写“没认出”，另外两格照常，位置不乱
        let view = view_of(&ocr("鼓声暴起 低音轰鸣", "？？新的", "终极九头蛇"), &d, 39).unwrap();
        assert_eq!(view.slots, [Some(Verdict::Ok), None, Some(Verdict::Best)]);
        assert_eq!(view.cells()[1], "没认出");
    }

    /// 玩家 2026-11-26：“队友有时候聊天框发送了海克斯也会触发海克斯推荐”。现在只截名字那一横条，而且名字要在同一行、
    /// 以中线对称等距排开：聊天里的名字（挤在左边、一行一个）、同一行写了好几个名字、不在同一行的（两行的）、排不齐的，都不算。
    #[test]
    fn chat_lines_and_stray_names_are_not_an_offer() {
        let d = dataset();
        // 聊天框（左下角，就算截到了也都挤在左边一处）：队友一行一个海克斯名
        let chat = vec![
            line("[队伍] 甲: 终极九头蛇", 0.13, 0.40),
            line("[队伍] 乙: 珠光护手", 0.13, 0.42),
            line("[队伍] 丙: 冰川之心", 0.13, 0.44),
            line("[队伍] 丁: 弹跳蜜蜂", 0.13, 0.46),
        ];
        assert_eq!(match_augments(&chat, &d).len(), 4, "名字都认得出");
        assert_eq!(offer_of(&chat, &d), None, "都挤在左边一处：不算");
        // 一行里写了三个名字：一行只认一个
        assert_eq!(
            offer_of(&[line("终极九头蛇 珠光护手 冰川之心", 0.3, 0.40)], &d),
            None
        );
        // 左、中、右各一个，但不在同一行（左边那个是聊天，低了一截）：不算
        let scattered = vec![
            line("终极九头蛇", 0.26, 0.47),
            line("珠光护手", 0.45, 0.395),
            line("冰川之心", 0.65, 0.36),
        ];
        assert_eq!(offer_of(&scattered, &d), None);
        // 四张分成两行（上面两个、下面两个，比如卡片说明里提到了别的名字）：一行只有两个、哪一行都排不成四张，不算
        let rows = vec![
            line("终极九头蛇", FOUR[0] - 0.05, 0.395),
            line("珠光护手", FOUR[1] - 0.05, 0.395),
            line("冰川之心", FOUR[2] - 0.05, 0.45),
            line("弹跳蜜蜂", FOUR[3] - 0.05, 0.45),
        ];
        assert_eq!(offer_of(&rows, &d), None);
        // 只认出两个、剩下那一格什么字都没有（卡片说明里提到了两个名字这类）：不算
        let two = vec![
            line("终极九头蛇", 0.26, 0.395),
            line("珠光护手", 0.65, 0.395),
        ];
        assert_eq!(offer_of(&two, &d), None);
        // 同一行三个名字，但没以中线对称（都偏在左边）：不是三选一也不是四选一
        let lopsided = vec![
            line("终极九头蛇", 0.10, 0.395),
            line("珠光护手", 0.25, 0.395),
            line("冰川之心", 0.40, 0.395),
        ];
        assert_eq!(offer_of(&lopsided, &d), None);
        // 真的三选一 + 聊天里的名字：照样认出三选一，聊天那行不进来
        let mut both = ocr("鼓声暴起 低音轰鸣", "弹跳蜜蜂", "终极九头蛇");
        both.push(line("[队伍] 甲: 珠光护手", 0.13, 0.46));
        assert_eq!(offer_of(&both, &d), Some(vec![Some(2), Some(4), Some(3)]));
        // 真的三选一 + 同一行别处的字碰巧对上了一个名字：去掉排不进的那个，还是三选一
        let mut stray = ocr("鼓声暴起 低音轰鸣", "弹跳蜜蜂", "终极九头蛇");
        stray.push(line("冰川之心", 0.86, 0.40));
        assert_eq!(offer_of(&stray, &d), Some(vec![Some(2), Some(4), Some(3)]));
        // 真的四选一 + 聊天里的名字：四张，聊天那行不进来
        let mut four = ocr4(["弹跳蜜蜂", "终极九头蛇", "鼓声暴起 低音轰鸣", "冰川之心"]);
        four.push(line("[队伍] 甲: 珠光护手", 0.13, 0.46));
        assert_eq!(
            offer_of(&four, &d),
            Some(vec![Some(4), Some(3), Some(2), Some(6)])
        );
    }

    #[test]
    fn when_all_three_are_weak_she_says_reroll() {
        let d = dataset();
        // 弹跳蜜蜂 48.3%、冰川之心 45%：都比平均低；首选还是分最高的那张（非要选就拿它）
        let lines = vec![
            line("冰川之心", 0.26, 0.395),
            line("弹跳蜜蜂", 0.45, 0.395),
            line("看不清的字", 0.65, 0.395),
        ];
        let view = view_of(&lines, &d, 39).unwrap();
        assert_eq!(view.best, Some(1));
        assert!(view.weak);
        // “都一般，可以刷新”不说位置；“刷新吧，不然选第几个”按第几个说，也不说一共几张（三张、四张一样）
        assert_eq!(
            view.voice(),
            ["都一般，可以刷新", "这几张都不太好，刷新吧，不然选第二个"]
        );
        for v in view.voice() {
            assert!(v.chars().count() <= Push::VOICE_MAX, "{v}");
        }
        // 四张都不好（四选一的最后一个是首选）：一样的说法
        let four = view_of(
            &ocr4(["冰川之心", "看不清", "糊了的字", "弹跳蜜蜂"]),
            &d,
            39,
        )
        .unwrap();
        assert!(four.weak);
        assert_eq!(four.voice()[1], "这几张都不太好，刷新吧，不然选第四个");
        // 一张有数据的都没有：不说
        let none = View {
            slots: vec![Some(Verdict::NoData), Some(Verdict::NoData), None],
            cards: vec![None, None, None],
            best: None,
            weak: false,
        };
        assert!(none.voice().is_empty());
        assert_eq!(none.cells()[0], "没数据");
    }

    /// 宿主的规矩（`agent/push.rs`）：同一个 key 再推一次，说法变了才开口。
    fn says(previous: Option<&[String]>, next: &[String]) -> bool {
        !next.is_empty() && previous.is_none_or(|p| p != next)
    }

    /// ADR 043 D7 的一次三选一，一帧一帧（中间夹着聊天框里的海克斯名，一律不算）：出现 → 卡片 + 语音；刷新了一张、首选没变
    /// → 原地换那一格、不说；首选变了 → 说；认成了四张 → 同一个 key 原地换成四格；卡片没了 4 秒 → 收起；之后再出来是新的
    /// 一次（新的 key）。
    #[test]
    fn one_offer_frame_by_frame() {
        let d = dataset();
        let t0 = Instant::now();
        // 第 s 秒（从 -10 秒起都行：前面先看两帧聊天）
        let at = |s: f64| t0 + Duration::from_secs_f64(s + 10.0);
        let mut tracker = Tracker::default();
        let mut pushes: Vec<(String, Vec<&str>, Vec<String>)> = Vec::new();
        let mut done: Vec<String> = Vec::new();
        let frame_of = |lines: Option<Vec<ScreenText>>| match lines {
            None => Frame::Unknown,
            Some(lines) => match view_of(&lines, &d, 39) {
                Some(view) => Frame::Cards(Box::new(view)),
                None => Frame::Nothing,
            },
        };
        // 看一帧（第 s 秒）：推的记进 pushes，收起的记进 done
        macro_rules! run {
            ($tracker:expr, $lines:expr, $s:expr) => {
                let s: f64 = $s;
                for step in $tracker.frame(frame_of($lines), at(s), (s * 1000.0) as i64, 9100001) {
                    match step {
                        Step::Push { key, view } => pushes.push((key, view.cells(), view.voice())),
                        Step::Done { key, .. } => done.push(key),
                    }
                }
            };
        }
        let chat = |text: &str| vec![line(&format!("[队伍] 甲: {text}"), 0.13, 0.42)];
        // 还没出三选一：队友在聊天里打了两个海克斯名（连着两帧一样）——不推
        run!(&mut tracker, Some(chat("终极九头蛇 弹跳蜜蜂")), -2.0);
        run!(&mut tracker, Some(chat("终极九头蛇 弹跳蜜蜂")), -1.0);
        assert!(
            pushes.is_empty() && !tracker.is_open(),
            "聊天里的名字不是三选一"
        );
        let first = ocr("鼓声暴起 低音轰鸣", "弹跳蜜蜂", "终极九头蛇");
        // 出现：第一帧先记下，第二帧一样才推（卡片 + 说“选第三个”）
        run!(&mut tracker, Some(first.clone()), 0.0);
        assert!(pushes.is_empty(), "只看到一次，不推");
        let mut noisy = first.clone();
        noisy.extend(chat("珠光护手"));
        run!(&mut tracker, Some(noisy), 1.0);
        assert_eq!(pushes.len(), 1, "聊天里多了一行不影响");
        let key = pushes[0].0.clone();
        assert_eq!(key, "augment:9100001:1000");
        assert!(says(None, &pushes[0].2));
        assert!(tracker.is_open());
        // 一样的画面：不再推
        run!(&mut tracker, Some(first.clone()), 2.0);
        assert_eq!(pushes.len(), 1);
        // 刷新了中间那张（弹跳蜜蜂 → 鼓声暴起：没数据也不比首选好）：刷新的动画那一帧中间没字（不是三选一的样子），不推；
        // 稳定以后推一次，同一个 key，只有中间那格变了，首选还在右边 → 说法一样（宿主不再说）
        run!(
            &mut tracker,
            Some(vec![
                line("鼓声暴起 低音轰鸣", 0.26, 0.395),
                line("终极九头蛇", 0.65, 0.395)
            ]),
            3.0
        );
        let refreshed = ocr("鼓声暴起 低音轰鸣", "鼓声暴起", "终极九头蛇");
        run!(&mut tracker, Some(refreshed.clone()), 4.0);
        assert_eq!(pushes.len(), 1, "动画里的一帧和下一帧不一样：都不推");
        run!(&mut tracker, Some(refreshed.clone()), 5.0);
        assert_eq!(pushes.len(), 2);
        assert_eq!(pushes[1].0, key, "同一次三选一：同一个 key（原地更新）");
        assert_eq!(pushes[1].1[0], pushes[0].1[0]);
        assert_eq!(pushes[1].1[2], pushes[0].1[2]);
        assert!(!says(Some(&pushes[0].2), &pushes[1].2), "首选没变：不说");
        // 刷新了右边那张（终极九头蛇 → 珠光护手 62%）：首选还在右边，但比它低得多的左边那张成了可刷新 → 原地换两格、不说
        let r2 = ocr("鼓声暴起 低音轰鸣", "鼓声暴起", "珠光护手");
        run!(&mut tracker, Some(r2.clone()), 6.0);
        run!(&mut tracker, Some(r2.clone()), 7.0);
        assert_eq!(pushes.len(), 3);
        assert_eq!(pushes[2].0, key);
        assert_eq!(pushes[2].1[0], "可刷新");
        assert_eq!(pushes[2].1[2], "首选");
        assert!(
            !says(Some(&pushes[1].2), &pushes[2].2),
            "首选还在右边：不说"
        );
        // 再刷新左边、右边：左边成了首选 → 说“选第一个”
        let r3 = ocr("珠光护手", "鼓声暴起", "冰川之心");
        run!(&mut tracker, Some(r3.clone()), 8.0);
        run!(&mut tracker, Some(r3.clone()), 9.0);
        assert_eq!(pushes.len(), 4);
        assert_eq!(pushes[3].0, key);
        assert_eq!(pushes[3].2[0], "选第一个");
        assert!(says(Some(&pushes[2].2), &pushes[3].2), "首选变了：说");
        // 认成了四张（三张、四张之间变了）：还是这一次，同一个 key 原地换成四格；首选还是第一个 → 说法一样，不说
        let four = ocr4(["珠光护手", "鼓声暴起", "冰川之心", "弹跳蜜蜂"]);
        run!(&mut tracker, Some(four.clone()), 9.5);
        run!(&mut tracker, Some(four.clone()), 9.8);
        assert_eq!(pushes.len(), 5);
        assert_eq!(pushes[4].0, key, "三张 ↔ 四张：同一个 key");
        assert_eq!(pushes[4].1, ["首选", "没数据", "可刷新", "可刷新"]);
        assert!(
            !says(Some(&pushes[3].2), &pushes[4].2),
            "首选还是第一个：不说"
        );
        // 游戏切走了一下（没截成）：什么都不改
        run!(&mut tracker, None, 10.0);
        assert!(tracker.is_open());
        // 选完了，卡片没了（只剩聊天）：不到 4 秒不收（识别偶尔漏一帧），到了 4 秒收起
        run!(&mut tracker, Some(chat("珠光护手")), 11.0);
        run!(&mut tracker, Some(Vec::new()), 12.0);
        assert!(done.is_empty());
        run!(&mut tracker, Some(Vec::new()), 14.0);
        assert_eq!(done, std::slice::from_ref(&key));
        assert!(!tracker.is_open());
        // 过一会儿又出三选一：新的一次，新的 key
        run!(&mut tracker, Some(first.clone()), 300.0);
        run!(&mut tracker, Some(first.clone()), 301.0);
        assert_eq!(pushes.len(), 6);
        assert_eq!(pushes[5].0, "augment:9100001:301000");
        assert_ne!(pushes[5].0, key);
        assert!(says(None, &pushes[5].2));
        // 这局结束了：开着的那一次收起（带着最后推出去的样子，卡片标“选好了”）
        match tracker.close() {
            Some(Step::Done { key, last }) => {
                assert_eq!(key, "augment:9100001:301000");
                assert_eq!(last.cells(), ["可选", "可刷新", "首选"]);
            }
            other => panic!("{other:?}"),
        }
    }

    /// 清单（ADR 043 D4、D7 的 2026-11-26 晚修订）：三选一是卡片 + 语音（还有“用不了”时的一行），要紧、要“用网上的统计”；
    /// 不再一直留在悬浮窗里（`until_done` 去掉了：卡片在聊天小窗里）。选英雄只有卡片、要客户端和统计。
    #[test]
    fn the_manifest_declares_both_pushes() {
        use kotone_plugin_api::{PushForm, PushPriority};
        let pick = crate::manifest().push_spec(PUSH_AUGMENT_PICK).unwrap();
        assert!(!pick.until_done);
        assert_eq!(pick.priority, PushPriority::High);
        assert_eq!(
            pick.forms,
            [PushForm::Card, PushForm::Voice, PushForm::Line]
        );
        assert_eq!(pick.needs, ["stats"]);
        assert!(crate::manifest().page(PAGE_AUGMENT_PICK).is_some());
        let select = crate::manifest()
            .push_spec(super::super::PUSH_CHAMP_SELECT)
            .unwrap();
        assert_eq!(select.forms, [PushForm::Card]);
        assert!(select.needs.contains(&"client".to_string()));
        assert!(!select.until_done);
    }

    /// 推出去的卡片：清单声明过的样子，宿主的检查过得去；结束时同一个 key、同样的说法、标上 ended。
    #[tokio::test]
    async fn the_pick_card_is_pushed_and_marked_ended() {
        let plugin = LolPlugin::new();
        let fake = FakeHost::new(LocalOcr::Ready {
            model: "m".into(),
            name: "m".into(),
        });
        let d = dataset();
        let view = view_of(&ocr("鼓声暴起 低音轰鸣", "弹跳蜜蜂", "终极九头蛇"), &d, 39).unwrap();
        let card = view.card(&d, None, json!({"id": 39, "short": "艾瑞莉娅"}), false);
        let push = pick_push("augment:1:2".into(), card.clone(), view.voice());
        fake.push(push.clone()).unwrap();
        plugin.pushes.lock().unwrap().pick.last_card =
            Some(("augment:1:2".into(), card, view.voice()));
        plugin.finish_pick(fake.as_ref(), "augment:1:2");
        let pushed = fake.pushed();
        assert_eq!(pushed.len(), 2);
        assert_eq!(pushed[1].key, pushed[0].key);
        assert_eq!(pushed[1].voice, pushed[0].voice, "说法不变：她不会再说");
        assert_eq!(pushed[1].card.as_ref().unwrap().data["ended"], true);
        assert_eq!(pushed[1].card.as_ref().unwrap().uri, PAGE_AUGMENT_PICK);
        assert!(pushed[1].line.is_none());
        assert_eq!(*fake.done.lock().unwrap(), ["augment:1:2"]);
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
        done: Mutex<Vec<String>>,
        enabled: bool,
    }

    impl FakeHost {
        fn new(ocr: LocalOcr) -> Arc<Self> {
            Arc::new(Self {
                ocr: Mutex::new(ocr),
                settings: Mutex::new(json!({})),
                pushes: Mutex::new(Vec::new()),
                done: Mutex::new(Vec::new()),
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
        fn push_done(&self, _trigger: &str, key: &str) {
            self.done.lock().unwrap().push(key.to_string());
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
