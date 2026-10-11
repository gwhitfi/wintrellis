//! Pure layout math: no Win32 calls, so it can be unit-tested anywhere.

use std::cmp::Reverse;

/// Most windows a layout tiles. Extra windows are left floating by the caller.
pub const MAX_TILED: usize = 6;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Where the dividing lines of the templates sit, as percentages. Every template
/// (2 to 6 windows) is built from the same four lines, so the same sizes carry over
/// when windows open or close. Lines a template doesn't have are ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Splits {
    /// Width of the big left window, as % of the whole area (2+ windows).
    pub left: i32,
    /// Height of the top-right window, as % of the right side (3+ windows).
    pub top_right: i32,
    /// Width of the bottom-right area's left part, as % of that area (4+ windows).
    pub corner_left: i32,
    /// Height of the upper row in the bottom-right area's right part (5 windows), or
    /// of the grid's top row (6 windows), as % of that area.
    pub corner_top: i32,
}

impl Splits {
    /// Every line in the middle.
    pub const EVEN: Splits = Splits {
        left: 50,
        top_right: 50,
        corner_left: 50,
        corner_top: 50,
    };
}

/// Returns one rectangle per slot for `count` windows (capped at `MAX_TILED`), with
/// `gap` pixels between windows and around the edge of `area`, and the dividing lines
/// at `splits`. Slot 0 is the big left pane, slot 1 the top right, and slots 2+ split
/// the bottom right.
pub fn layout(area: Rect, count: usize, gap: i32, splits: Splits) -> Vec<Rect> {
    // Shrinking the area by half a gap, then every slot by half a gap, puts a full
    // gap both between neighbors (half from each side) and at the screen edges.
    let half = gap / 2;
    split(inset(area, gap - half), count, splits)
        .into_iter()
        .map(|slot| inset(slot, half))
        .collect()
}

/// Splits `area` into slots with no gaps.
fn split(area: Rect, count: usize, splits: Splits) -> Vec<Rect> {
    match count.min(MAX_TILED) {
        0 => vec![],
        1 => vec![area],
        n => {
            let (left, right) = split_columns(area, splits.left);
            if n == 2 {
                return vec![left, right];
            }
            let (top_right, bottom_right) = split_rows(right, splits.top_right);
            if n == 3 {
                return vec![left, top_right, bottom_right];
            }
            let (corner_left, corner_right) = split_columns(bottom_right, splits.corner_left);
            match n {
                4 => vec![left, top_right, corner_left, corner_right],
                5 => {
                    let (upper, lower) = split_rows(corner_right, splits.corner_top);
                    vec![left, top_right, corner_left, upper, lower]
                }
                _ => {
                    // 2x2 grid. Numbered so going from 5 to 6 windows only slot 2 changes.
                    let (top_left, bottom_left) = split_rows(corner_left, splits.corner_top);
                    let (top_right2, bottom_right2) = split_rows(corner_right, splits.corner_top);
                    vec![
                        left,
                        top_right,
                        top_left,
                        top_right2,
                        bottom_right2,
                        bottom_left,
                    ]
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// How far one resize step moves a dividing line, in percent.
const RESIZE_STEP: i32 = 10;
/// No dividing line goes closer than this to either end, so no window disappears.
const MIN_PERCENT: i32 = 10;

/// One of the four dividing lines in `Splits`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Line {
    Left,
    TopRight,
    CornerLeft,
    CornerTop,
}

/// The dividing lines along each side of a slot: [left, right, top, bottom], `None`
/// where the slot touches the edge of the screen.
fn borders(count: usize, slot: usize) -> [Option<Line>; 4] {
    use Line::*;
    let a = Some(Left);
    // B, C and D only exist from 3, 4 and 5 windows on.
    let b = (count >= 3).then_some(TopRight);
    match (count.min(MAX_TILED), slot) {
        (0 | 1, _) => [None; 4],
        (_, 0) => [None, a, None, None],
        (_, 1) => [a, None, None, b],
        (3, 2) => [a, None, b, None],
        (4 | 5, 2) => [a, Some(CornerLeft), b, None],
        (4, 3) => [Some(CornerLeft), None, b, None],
        (5, 3) => [Some(CornerLeft), None, b, Some(CornerTop)],
        (5, 4) => [Some(CornerLeft), None, Some(CornerTop), None],
        // 6 windows: 2 grid top-left, 3 top-right, 4 bottom-right, 5 bottom-left.
        (6, 2) => [a, Some(CornerLeft), b, Some(CornerTop)],
        (6, 3) => [Some(CornerLeft), None, b, Some(CornerTop)],
        (6, 4) => [Some(CornerLeft), None, Some(CornerTop), None],
        (6, 5) => [a, Some(CornerLeft), Some(CornerTop), None],
        _ => [None; 4],
    }
}

/// Resizes the window in `slot` (of `count` tiled windows) by moving one of its
/// borders a step in `dir`: the border on that side if it has one (the window grows),
/// otherwise the one on the opposite side (it shrinks). Returns the new splits, or
/// `None` if there's no border to move that way or it's already as far as it goes.
pub fn resize(splits: Splits, count: usize, slot: usize, dir: Direction) -> Option<Splits> {
    let [left, right, top, bottom] = borders(count, slot);
    let (line, step) = match dir {
        Direction::Left => (left.or(right)?, -RESIZE_STEP),
        Direction::Right => (right.or(left)?, RESIZE_STEP),
        Direction::Up => (top.or(bottom)?, -RESIZE_STEP),
        Direction::Down => (bottom.or(top)?, RESIZE_STEP),
    };
    let mut resized = splits;
    let percent = match line {
        Line::Left => &mut resized.left,
        Line::TopRight => &mut resized.top_right,
        Line::CornerLeft => &mut resized.corner_left,
        Line::CornerTop => &mut resized.corner_top,
    };
    *percent = (*percent + step).clamp(MIN_PERCENT, 100 - MIN_PERCENT);
    (resized != splits).then_some(resized)
}

/// Index of the rectangle in `candidates` that is the best move from `from` in `dir`,
/// or `None` if nothing lies that way. Only rectangles that line up with `from` count
/// (share some of its height for left/right, or width for up/down). Among those: the
/// nearest, then the one sharing the most, then the lowest index (earliest slot).
pub fn neighbor(from: Rect, candidates: &[Rect], dir: Direction) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter(|&(_, &r)| is_ahead(from, r, dir))
        .map(|(i, &r)| (i, measure(from, r, dir)))
        .filter(|&(_, (_, shared))| shared > 0)
        .min_by_key(|&(i, (distance, shared))| (distance, -shared, i))
        .map(|(i, _)| i)
}

/// True if `r`'s center is past `from`'s center in `dir`. (Centers are doubled to
/// avoid rounding.) This also excludes `from` itself.
fn is_ahead(from: Rect, r: Rect, dir: Direction) -> bool {
    let (fx, fy) = (2 * from.x + from.w, 2 * from.y + from.h);
    let (rx, ry) = (2 * r.x + r.w, 2 * r.y + r.h);
    match dir {
        Direction::Left => rx < fx,
        Direction::Right => rx > fx,
        Direction::Up => ry < fy,
        Direction::Down => ry > fy,
    }
}

/// (empty space between the two edges facing each other, length of shared side).
fn measure(from: Rect, r: Rect, dir: Direction) -> (i32, i32) {
    let (distance, shared) = match dir {
        Direction::Left => (from.x - (r.x + r.w), shared(from.y, from.h, r.y, r.h)),
        Direction::Right => (r.x - (from.x + from.w), shared(from.y, from.h, r.y, r.h)),
        Direction::Up => (from.y - (r.y + r.h), shared(from.x, from.w, r.x, r.w)),
        Direction::Down => (r.y - (from.y + from.h), shared(from.x, from.w, r.x, r.w)),
    };
    // Overlapping rectangles (e.g. a floating window over tiles) count as touching.
    (distance.max(0), shared)
}

/// How much two spans (start, length) overlap.
fn shared(a: i32, a_len: i32, b: i32, b_len: i32) -> i32 {
    ((a + a_len).min(b + b_len) - a.max(b)).max(0)
}

/// Orders windows so each keeps the slot it already mostly covers on screen. `windows`
/// are their current rectangles. Returns indices into `windows`: the window for slot 0,
/// then slot 1, and so on, followed by any windows left without a slot, in original order.
pub fn match_slots(slots: &[Rect], windows: &[Rect]) -> Vec<usize> {
    let mut remaining: Vec<usize> = (0..windows.len()).collect();
    let mut order = Vec::with_capacity(windows.len());
    for &slot in slots {
        // Most overlap wins; on a tie, the window that came first.
        let best = remaining
            .iter()
            .enumerate()
            .max_by_key(|&(pos, &i)| (overlap_area(slot, windows[i]), Reverse(pos)))
            .map(|(pos, _)| pos);
        let Some(best) = best else {
            break;
        };
        order.push(remaining.remove(best));
    }
    order.extend(remaining);
    order
}

fn overlap_area(a: Rect, b: Rect) -> i64 {
    shared(a.x, a.w, b.x, b.w) as i64 * shared(a.y, a.h, b.y, b.h) as i64
}

/// Shrinks a rectangle by `by` pixels on every side.
fn inset(r: Rect, by: i32) -> Rect {
    Rect {
        x: r.x + by,
        y: r.y + by,
        w: r.w - 2 * by,
        h: r.h - 2 * by,
    }
}

/// Splits into a left part `percent`% wide and a right part. A leftover pixel from
/// rounding goes to the right part.
fn split_columns(r: Rect, percent: i32) -> (Rect, Rect) {
    let left = r.w * percent / 100;
    (
        Rect { w: left, ..r },
        Rect {
            x: r.x + left,
            w: r.w - left,
            ..r
        },
    )
}

/// Splits into a top part `percent`% tall and a bottom part. A leftover pixel from
/// rounding goes to the bottom part.
fn split_rows(r: Rect, percent: i32) -> (Rect, Rect) {
    let top = r.h * percent / 100;
    (
        Rect { h: top, ..r },
        Rect {
            y: r.y + top,
            h: r.h - top,
            ..r
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // Odd size and non-zero origin to catch rounding and offset mistakes.
    const AREA: Rect = Rect {
        x: 100,
        y: 50,
        w: 1919,
        h: 1079,
    };

    fn overlaps(a: Rect, b: Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    #[test]
    fn slot_count_matches_window_count_up_to_max() {
        for count in 0..=8 {
            assert_eq!(
                layout(AREA, count, 0, Splits::EVEN).len(),
                count.min(MAX_TILED)
            );
        }
    }

    #[test]
    fn slots_fill_the_area_exactly_without_overlapping() {
        for count in 1..=MAX_TILED {
            let slots = layout(AREA, count, 0, Splits::EVEN);
            let total: i32 = slots.iter().map(|r| r.w * r.h).sum();
            assert_eq!(total, AREA.w * AREA.h, "count {count}: gaps or overflow");
            for (i, &a) in slots.iter().enumerate() {
                assert!(a.x >= AREA.x && a.y >= AREA.y, "count {count} slot {i}");
                assert!(a.x + a.w <= AREA.x + AREA.w && a.y + a.h <= AREA.y + AREA.h);
                for &b in &slots[i + 1..] {
                    assert!(!overlaps(a, b), "count {count}: {a:?} overlaps {b:?}");
                }
            }
        }
    }

    #[test]
    fn adding_a_window_changes_exactly_one_existing_slot() {
        for count in 1..MAX_TILED {
            let before = layout(AREA, count, 0, Splits::EVEN);
            let after = layout(AREA, count + 1, 0, Splits::EVEN);
            let changed = (0..count).filter(|&i| before[i] != after[i]).count();
            assert_eq!(changed, 1, "going from {count} to {} windows", count + 1);
        }
    }

    #[test]
    fn six_windows_match_the_sketch() {
        let s = layout(
            Rect {
                x: 0,
                y: 0,
                w: 1000,
                h: 800,
            },
            6,
            0,
            Splits::EVEN,
        );
        let r = |x, y, w, h| Rect { x, y, w, h };
        assert_eq!(s[0], r(0, 0, 500, 800)); // big left pane
        assert_eq!(s[1], r(500, 0, 500, 400)); // top right
        assert_eq!(s[2], r(500, 400, 250, 200)); // grid top-left
        assert_eq!(s[3], r(750, 400, 250, 200)); // grid top-right
        assert_eq!(s[4], r(750, 600, 250, 200)); // grid bottom-right
        assert_eq!(s[5], r(500, 600, 250, 200)); // grid bottom-left
    }

    #[test]
    fn splits_move_the_dividing_lines() {
        let area = Rect {
            x: 0,
            y: 0,
            w: 1000,
            h: 800,
        };
        let splits = Splits {
            left: 30,
            top_right: 70,
            corner_left: 40,
            corner_top: 25,
        };
        let s = layout(area, 6, 0, splits);
        let r = |x, y, w, h| Rect { x, y, w, h };
        assert_eq!(s[0], r(0, 0, 300, 800)); // 30% of 1000 wide
        assert_eq!(s[1], r(300, 0, 700, 560)); // 70% of 800 tall
        assert_eq!(s[2], r(300, 560, 280, 60)); // 40% of 700 wide, 25% of 240 tall
        assert_eq!(s[3], r(580, 560, 420, 60));
        assert_eq!(s[4], r(580, 620, 420, 180));
        assert_eq!(s[5], r(300, 620, 280, 180));
    }

    #[test]
    fn uneven_splits_still_fill_the_area_exactly() {
        let splits = Splits {
            left: 10,
            top_right: 90,
            corner_left: 33,
            corner_top: 67,
        };
        for count in 1..=MAX_TILED {
            let slots = layout(AREA, count, 0, splits);
            let total: i32 = slots.iter().map(|r| r.w * r.h).sum();
            assert_eq!(total, AREA.w * AREA.h, "count {count}");
        }
    }

    #[test]
    fn resizing_two_windows() {
        use Direction::*;
        // Right window: Left grows it (the line moves left), Right shrinks it.
        assert_eq!(resize(Splits::EVEN, 2, 1, Left).map(|s| s.left), Some(40));
        assert_eq!(resize(Splits::EVEN, 2, 1, Right).map(|s| s.left), Some(60));
        // Left window: Right grows it, Left shrinks it.
        assert_eq!(resize(Splits::EVEN, 2, 0, Right).map(|s| s.left), Some(60));
        assert_eq!(resize(Splits::EVEN, 2, 0, Left).map(|s| s.left), Some(40));
        // No horizontal lines with two windows.
        assert_eq!(resize(Splits::EVEN, 2, 1, Up), None);
    }

    #[test]
    fn resizing_four_windows() {
        use Direction::*;
        // Top right growing left narrows the big window and widens the whole right side.
        assert_eq!(resize(Splits::EVEN, 4, 1, Left).map(|s| s.left), Some(40));
        // Top right growing down makes the bottom two shorter.
        assert_eq!(
            resize(Splits::EVEN, 4, 1, Down).map(|s| s.top_right),
            Some(60)
        );
        // Bottom-left has a line on both sides: each arrow grows it that way.
        assert_eq!(resize(Splits::EVEN, 4, 2, Left).map(|s| s.left), Some(40));
        assert_eq!(
            resize(Splits::EVEN, 4, 2, Right).map(|s| s.corner_left),
            Some(60)
        );
    }

    #[test]
    fn resizing_the_six_window_grid() {
        use Direction::*;
        assert_eq!(
            resize(Splits::EVEN, 6, 2, Down).map(|s| s.corner_top),
            Some(60)
        );
        assert_eq!(
            resize(Splits::EVEN, 6, 5, Up).map(|s| s.corner_top),
            Some(40)
        );
        assert_eq!(
            resize(Splits::EVEN, 6, 4, Left).map(|s| s.corner_left),
            Some(40)
        );
    }

    #[test]
    fn resizing_stops_at_the_limits() {
        use Direction::*;
        let narrow = Splits {
            left: 10,
            ..Splits::EVEN
        };
        assert_eq!(resize(narrow, 2, 1, Left), None);
        assert_eq!(resize(narrow, 2, 1, Right).map(|s| s.left), Some(20));
        assert_eq!(resize(Splits::EVEN, 1, 0, Left), None);
    }

    #[test]
    fn every_resize_moves_a_border_of_the_window_the_right_way() {
        use Direction::*;
        for count in 2..=MAX_TILED {
            for slot in 0..count {
                for dir in [Left, Right, Up, Down] {
                    let Some(resized) = resize(Splits::EVEN, count, slot, dir) else {
                        continue;
                    };
                    let a = layout(AREA, count, 0, Splits::EVEN)[slot];
                    let b = layout(AREA, count, 0, resized)[slot];
                    let moved = match dir {
                        Left => b.x < a.x || b.x + b.w < a.x + a.w,
                        Right => b.x > a.x || b.x + b.w > a.x + a.w,
                        Up => b.y < a.y || b.y + b.h < a.y + a.h,
                        Down => b.y > a.y || b.y + b.h > a.y + a.h,
                    };
                    assert!(moved, "count {count} slot {slot} {dir:?}: {a:?} -> {b:?}");
                }
            }
        }
    }

    #[test]
    fn every_border_on_screen_can_be_moved() {
        use Direction::*;
        // Any window that doesn't touch the screen edge on a side can be resized
        // toward that side, so no border is accidentally left out of `borders`.
        for count in 2..=MAX_TILED {
            for slot in 0..count {
                let r = layout(AREA, count, 0, Splits::EVEN)[slot];
                let inner = [
                    (Left, r.x > AREA.x),
                    (Right, r.x + r.w < AREA.x + AREA.w),
                    (Up, r.y > AREA.y),
                    (Down, r.y + r.h < AREA.y + AREA.h),
                ];
                for (dir, has_neighbor) in inner {
                    let grows = resize(Splits::EVEN, count, slot, dir).is_some();
                    assert!(!has_neighbor || grows, "count {count} slot {slot} {dir:?}");
                }
            }
        }
    }

    #[test]
    fn two_windows_with_gaps() {
        let area = Rect {
            x: 0,
            y: 0,
            w: 1000,
            h: 800,
        };
        let s = layout(area, 2, 10, Splits::EVEN);
        let r = |x, y, w, h| Rect { x, y, w, h };
        assert_eq!(s[0], r(10, 10, 485, 780));
        assert_eq!(s[1], r(505, 10, 485, 780));
    }

    #[test]
    fn gaps_separate_every_slot_from_edges_and_neighbors() {
        let gap = 10;
        for count in 1..=MAX_TILED {
            let slots = layout(AREA, count, gap, Splits::EVEN);
            for (i, &a) in slots.iter().enumerate() {
                assert!(
                    a.x - AREA.x >= gap && a.y - AREA.y >= gap,
                    "count {count} slot {i}"
                );
                assert!(
                    AREA.x + AREA.w - (a.x + a.w) >= gap,
                    "count {count} slot {i}"
                );
                assert!(
                    AREA.y + AREA.h - (a.y + a.h) >= gap,
                    "count {count} slot {i}"
                );
                // Growing a slot by `gap` on every side must not reach any other slot.
                for &b in &slots[i + 1..] {
                    assert!(
                        !overlaps(inset(a, -gap), b),
                        "count {count}: {a:?} too close to {b:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn neighbors_in_six_window_layout() {
        use Direction::*;
        // Slots: 0 left pane, 1 top right, 2/3 grid top, 4 grid bottom-right, 5 grid bottom-left.
        let s = layout(AREA, 6, 8, Splits::EVEN);
        let cases = [
            (0, Right, Some(1)), // top right shares the most height with the left pane
            (0, Left, None),
            (0, Up, None),
            (1, Down, Some(2)), // grid top-left and top-right tie; lower slot wins
            (1, Left, Some(0)),
            (2, Left, Some(0)),
            (2, Up, Some(1)),
            (2, Right, Some(3)),
            (2, Down, Some(5)),
            (3, Down, Some(4)),
            (3, Up, Some(1)),
            (4, Left, Some(5)),
            (5, Up, Some(2)),
            (5, Left, Some(0)),
            (4, Right, None),
        ];
        for (from, dir, expected) in cases {
            assert_eq!(
                neighbor(s[from], &s, dir),
                expected,
                "from slot {from} going {dir:?}"
            );
        }
    }

    #[test]
    fn neighbors_in_four_window_layout() {
        use Direction::*;
        let s = layout(AREA, 4, 8, Splits::EVEN);
        assert_eq!(neighbor(s[3], &s, Up), Some(1));
        assert_eq!(neighbor(s[3], &s, Left), Some(2));
        assert_eq!(neighbor(s[2], &s, Left), Some(0));
        assert_eq!(neighbor(s[1], &s, Down), Some(2));
    }

    #[test]
    fn neighbor_from_a_floating_window_over_the_tiles() {
        let s = layout(AREA, 2, 8, Splits::EVEN);
        // A window floating in the middle of the screen, overlapping both tiles.
        let floating = Rect {
            x: AREA.x + 700,
            y: AREA.y + 300,
            w: 500,
            h: 400,
        };
        assert_eq!(neighbor(floating, &s, Direction::Left), Some(0));
        assert_eq!(neighbor(floating, &s, Direction::Right), Some(1));
    }

    #[test]
    fn existing_arrangement_is_kept_on_startup() {
        // The reported bug: Zed, Firefox, Alacritty, PowerShell were in slots 0-3, but
        // Windows listed them in focus order (Alacritty before Firefox).
        let slots = layout(AREA, 4, 8, Splits::EVEN);
        let zed = slots[0];
        let firefox = slots[1];
        let alacritty = slots[2];
        let powershell = slots[3];
        let in_focus_order = [zed, alacritty, firefox, powershell];
        assert_eq!(match_slots(&slots, &in_focus_order), vec![0, 2, 1, 3]);
    }

    #[test]
    fn roughly_placed_windows_get_the_slot_they_mostly_cover() {
        let slots = layout(AREA, 2, 8, Splits::EVEN);
        // Right window a bit off its slot, left window hanging past the middle.
        let right = Rect {
            x: slots[1].x + 30,
            y: slots[1].y + 20,
            ..slots[1]
        };
        let left = Rect {
            w: slots[0].w + 100,
            ..slots[0]
        };
        assert_eq!(match_slots(&slots, &[right, left]), vec![1, 0]);
    }

    #[test]
    fn windows_without_a_slot_keep_their_order_at_the_end() {
        let slots = layout(AREA, 1, 8, Splits::EVEN);
        let elsewhere = Rect::default();
        assert_eq!(
            match_slots(&slots, &[elsewhere, slots[0], elsewhere]),
            vec![1, 0, 2]
        );
    }
}
