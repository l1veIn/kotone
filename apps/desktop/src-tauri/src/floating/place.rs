//! 悬浮窗摆在哪（纯计算，物理像素；ADR 043 D6）。
//!
//! - **没拖过**：贴着设置里的角（`assistant.bubble.corner` + `offsetY`），让开同一个角上的输入条（屏幕角落分配，
//!   `screen_corner::stack`）。
//! - **拖过**：照记下的位置（`assistant.bubble.position`，[`FloatingPosition`]）：窗口的哪个角贴在工作区里的哪一点，
//!   按工作区的比例记，所以换一块屏幕、换分辨率照样用（[`remember`] / [`at`]）。拖到哪里不和输入条排队。
//!
//! 两种都照 [mini-yifan/dsh-orb-cordis](https://github.com/mini-yifan/dsh-orb-cordis)（MIT，`packages/helper/src/geometry.ts`
//! 的 `FloatingPlacement`）：窗口变大变小（气泡 ↔ 小窗、卡片变高）时贴着的那个角不动，从它往屏幕里长
//! （`expandDirection` / `ballOriginFromWindow`）；松手时整个拉回工作区里（`clampedBallOrigin`）。它“松手贴边收成细条”
//! 和“悬停展开”不照搬：后者在游戏里会抢焦点（2026-11-22 调研第四节）。

use kotone_core::assistant::{BubbleCorner, FloatingPosition};

use crate::screen_corner::{Corner, Rect};

/// 设置里的角 → 摆位用的角。
pub fn corner_of(corner: BubbleCorner) -> Corner {
    match corner {
        BubbleCorner::BottomRight => Corner::BottomRight,
        BubbleCorner::BottomLeft => Corner::BottomLeft,
        BubbleCorner::TopRight => Corner::TopRight,
        BubbleCorner::TopLeft => Corner::TopLeft,
    }
}

fn width(r: &Rect) -> i64 {
    (r.right - r.left).max(1) as i64
}

fn height(r: &Rect) -> i64 {
    (r.bottom - r.top).max(1) as i64
}

/// 拖完松手：窗口 `rect` 的中心在工作区 `area` 的哪一块，就贴着哪个角（以后从这个角往屏幕里长）；记下这个角在工作区里的
/// 位置（万分之一，四舍五入）。
pub fn remember(rect: Rect, area: Rect) -> FloatingPosition {
    let right = rect.left + rect.right > area.left + area.right;
    let bottom = rect.top + rect.bottom > area.top + area.bottom;
    let corner = match (right, bottom) {
        (true, true) => BubbleCorner::BottomRight,
        (false, true) => BubbleCorner::BottomLeft,
        (true, false) => BubbleCorner::TopRight,
        (false, false) => BubbleCorner::TopLeft,
    };
    let x = if right { rect.right } else { rect.left };
    let y = if bottom { rect.bottom } else { rect.top };
    let scale = FloatingPosition::SCALE as i64;
    let part = |at: i32, start: i32, size: i64| -> u32 {
        let offset = (at - start) as i64;
        ((offset * scale + size / 2) / size).clamp(0, scale) as u32
    };
    FloatingPosition {
        corner,
        x: part(x, area.left, width(&area)),
        y: part(y, area.top, height(&area)),
    }
}

/// 照记下的位置摆一个 `size` 大的窗口：`position.corner` 那个角贴在记下的那一点，往另外三边长；出了工作区就整个拉回来
/// （屏幕比窗口还小时贴着左上）。
pub fn at(position: FloatingPosition, area: Rect, size: (i32, i32)) -> Rect {
    let scale = FloatingPosition::SCALE as i64;
    let x = position.x.min(FloatingPosition::SCALE) as i64;
    let y = position.y.min(FloatingPosition::SCALE) as i64;
    let ax = area.left + ((x * width(&area) + scale / 2) / scale) as i32;
    let ay = area.top + ((y * height(&area) + scale / 2) / scale) as i32;
    let corner = corner_of(position.corner);
    let right = matches!(corner, Corner::TopRight | Corner::BottomRight);
    let bottom = matches!(corner, Corner::BottomLeft | Corner::BottomRight);
    let left = if right { ax - size.0 } else { ax };
    let top = if bottom { ay - size.1 } else { ay };
    let left = left.clamp(area.left, (area.right - size.0).max(area.left));
    let top = top.clamp(area.top, (area.bottom - size.1).max(area.top));
    Rect::at(left, top, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-25 真机那块屏：3840 × 2160、任务栏 72 高（物理像素）。
    const BIG: Rect = Rect {
        left: 0,
        top: 0,
        right: 3840,
        bottom: 2088,
    };
    /// 右边的第二块屏：1920 × 1080、任务栏 48 高，左边接着第一块。
    const SECOND: Rect = Rect {
        left: 3840,
        top: 0,
        right: 5760,
        bottom: 1032,
    };

    fn near(a: i32, b: i32) -> bool {
        (a - b).abs() <= 1
    }

    /// 拖到哪里，下次就从哪里出现：同一块屏上记下再摆回来，四个方向都落在原处（差不到 1 个像素）。
    #[test]
    fn a_dragged_window_comes_back_where_it_was_left() {
        let size = (560, 150);
        for (left, top, corner) in [
            (3000, 1500, BubbleCorner::BottomRight),
            (200, 1700, BubbleCorner::BottomLeft),
            (3100, 120, BubbleCorner::TopRight),
            (40, 30, BubbleCorner::TopLeft),
        ] {
            let rect = Rect::at(left, top, size);
            let p = remember(rect, BIG);
            assert_eq!(p.corner, corner, "{rect:?}");
            let back = at(p, BIG, size);
            assert!(
                near(back.left, rect.left) && near(back.top, rect.top),
                "{rect:?} → {p:?} → {back:?}"
            );
        }
    }

    /// 气泡变成小窗（更高、更宽）：贴着的那个角不动，从它往屏幕里长。
    #[test]
    fn growing_keeps_the_anchored_corner_still() {
        let bubble = Rect::at(2900, 1400, (540, 140));
        let p = remember(bubble, BIG);
        assert_eq!(p.corner, BubbleCorner::BottomRight);
        let mini = at(p, BIG, (600, 700));
        assert!(near(mini.right, bubble.right) && near(mini.bottom, bubble.bottom));
        assert!(mini.top < bubble.top && mini.left < bubble.left);
        // 左上角贴着的：往右下长
        let top_left = Rect::at(100, 90, (540, 140));
        let p = remember(top_left, BIG);
        let mini = at(p, BIG, (600, 700));
        assert!(near(mini.left, top_left.left) && near(mini.top, top_left.top));
    }

    /// 换一块屏（大小、位置都不一样）：落在那块屏工作区里同样比例的地方，贴着同一个角；窗口大小照那块屏的缩放给。
    #[test]
    fn the_remembered_place_carries_over_to_another_monitor() {
        // 在大屏上拖到右下、离右边 10%、离下边 20% 的地方
        let rect = Rect::at(3456 - 540, 1670 - 140, (540, 140));
        let p = remember(rect, BIG);
        assert_eq!(p.corner, BubbleCorner::BottomRight);
        assert_eq!((p.x, p.y), (9000, 7998));
        // 游戏在第二块屏（缩放 100%：窗口 360 × 93）
        let size = (360, 93);
        let there = at(p, SECOND, size);
        assert_eq!(there.right, 3840 + 1728, "离右边 10%");
        assert!(near(there.bottom, 825), "离下边 20%：{there:?}");
        assert_eq!((there.right - there.left, there.bottom - there.top), size);
    }

    /// 出了工作区就拉回来：窗口比记下时大、记下的点在边上、屏幕比窗口还小。
    #[test]
    fn never_leaves_the_work_area() {
        let p = FloatingPosition {
            corner: BubbleCorner::TopRight,
            x: 10_000,
            y: 9_900,
        };
        let r = at(p, BIG, (800, 900));
        assert_eq!(r.right, BIG.right);
        assert_eq!(r.bottom, BIG.bottom, "往下长出了工作区：拉回来");
        let tiny = Rect {
            left: 0,
            top: 0,
            right: 300,
            bottom: 200,
        };
        let r = at(p, tiny, (800, 900));
        assert_eq!((r.left, r.top), (0, 0));
        // 设置里写坏了的值（大过万分之一万）按边上算
        let wild = FloatingPosition {
            corner: BubbleCorner::BottomLeft,
            x: 60_000,
            y: 60_000,
        };
        let r = at(wild, SECOND, (360, 93));
        assert_eq!((r.right, r.bottom), (SECOND.right, SECOND.bottom));
    }
}
