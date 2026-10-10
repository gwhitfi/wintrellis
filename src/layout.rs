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

/// Returns one rectangle per slot for `count` windows (capped at `MAX_TILED`), with
/// `gap` pixels between windows and around the edge of `area`.
/// Slot 0 is the big left pane, slot 1 the top right, and slots 2+ split the bottom right.
pub fn layout(area: Rect, count: usize, gap: i32) -> Vec<Rect> {
    // Shrinking the area by half a gap, then every slot by half a gap, puts a full
    // gap both between neighbors (half from each side) and at the screen edges.
    let half = gap / 2;
    split(inset(area, gap - half), count)
        .into_iter()
        .map(|slot| inset(slot, half))
        .collect()
}

/// Splits `area` into slots with no gaps.
fn split(area: Rect, count: usize) -> Vec<Rect> {
    match count.min(MAX_TILED) {
        0 => vec![],
        1 => vec![area],
        n => {
            let (left, right) = split_columns(area);
            if n == 2 {
                return vec![left, right];
            }
            let (top_right, bottom_right) = split_rows(right);
            if n == 3 {
                return vec![left, top_right, bottom_right];
            }
            let (corner_left, corner_right) = split_columns(bottom_right);
            match n {
                4 => vec![left, top_right, corner_left, corner_right],
                5 => {
                    let (upper, lower) = split_rows(corner_right);
                    vec![left, top_right, corner_left, upper, lower]
                }
                _ => {
                    // 2x2 grid. Numbered so going from 5 to 6 windows only slot 2 changes.
                    let (top_left, bottom_left) = split_rows(corner_left);
                    let (top_right2, bottom_right2) = split_rows(corner_right);
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

/// Splits into left and right halves. An odd pixel goes to the right half.
fn split_columns(r: Rect) -> (Rect, Rect) {
    let half = r.w / 2;
    (
        Rect { w: half, ..r },
        Rect {
            x: r.x + half,
            w: r.w - half,
            ..r
        },
    )
}

/// Splits into top and bottom halves. An odd pixel goes to the bottom half.
fn split_rows(r: Rect) -> (Rect, Rect) {
    let half = r.h / 2;
    (
        Rect { h: half, ..r },
        Rect {
            y: r.y + half,
            h: r.h - half,
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
            assert_eq!(layout(AREA, count, 0).len(), count.min(MAX_TILED));
        }
    }

    #[test]
    fn slots_fill_the_area_exactly_without_overlapping() {
        for count in 1..=MAX_TILED {
            let slots = layout(AREA, count, 0);
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
            let before = layout(AREA, count, 0);
            let after = layout(AREA, count + 1, 0);
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
    fn two_windows_with_gaps() {
        let area = Rect {
            x: 0,
            y: 0,
            w: 1000,
            h: 800,
        };
        let s = layout(area, 2, 10);
        let r = |x, y, w, h| Rect { x, y, w, h };
        assert_eq!(s[0], r(10, 10, 485, 780));
        assert_eq!(s[1], r(505, 10, 485, 780));
    }

    #[test]
    fn gaps_separate_every_slot_from_edges_and_neighbors() {
        let gap = 10;
        for count in 1..=MAX_TILED {
            let slots = layout(AREA, count, gap);
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
        let s = layout(AREA, 6, 8);
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
        let s = layout(AREA, 4, 8);
        assert_eq!(neighbor(s[3], &s, Up), Some(1));
        assert_eq!(neighbor(s[3], &s, Left), Some(2));
        assert_eq!(neighbor(s[2], &s, Left), Some(0));
        assert_eq!(neighbor(s[1], &s, Down), Some(2));
    }

    #[test]
    fn neighbor_from_a_floating_window_over_the_tiles() {
        let s = layout(AREA, 2, 8);
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
        let slots = layout(AREA, 4, 8);
        let zed = slots[0];
        let firefox = slots[1];
        let alacritty = slots[2];
        let powershell = slots[3];
        let in_focus_order = [zed, alacritty, firefox, powershell];
        assert_eq!(match_slots(&slots, &in_focus_order), vec![0, 2, 1, 3]);
    }

    #[test]
    fn roughly_placed_windows_get_the_slot_they_mostly_cover() {
        let slots = layout(AREA, 2, 8);
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
        let slots = layout(AREA, 1, 8);
        let elsewhere = Rect::default();
        assert_eq!(
            match_slots(&slots, &[elsewhere, slots[0], elsewhere]),
            vec![1, 0, 2]
        );
    }
}
