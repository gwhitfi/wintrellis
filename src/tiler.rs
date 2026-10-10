use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BeginDeferWindowPos, DeferWindowPos, EndDeferWindowPos, GetWindowRect, IsZoomed, SW_RESTORE,
    SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, ShowWindow,
};

use crate::layout::{self, Rect};
use crate::{settings, window};

thread_local! {
    /// Overflow windows we've already centered. Each is centered once, when it first
    /// overflows, so later retiles don't yank it back after it's moved by hand.
    static CENTERED: RefCell<Vec<HWND>> = const { RefCell::new(Vec::new()) };
}

/// Moves every window into its slot. Windows are tiled on whichever monitor they're
/// currently on, in list order. Windows beyond `layout::MAX_TILED` float, centered.
pub fn retile(windows: &[HWND]) {
    let mut moves = Vec::new();
    let mut overflow = Vec::new();
    for (monitor, group) in group_by_monitor(windows) {
        let Some(area) = work_area(monitor) else {
            continue;
        };
        let slots = layout::layout(area, group.len(), settings::gap());
        for (&hwnd, slot) in group.iter().zip(&slots) {
            moves.push((hwnd, outer_rect(hwnd, *slot)));
        }
        for &hwnd in &group[slots.len()..] {
            if !CENTERED.with_borrow(|centered| centered.contains(&hwnd))
                && let Some(slot) = centered_slot(hwnd, area)
            {
                moves.push((hwnd, outer_rect(hwnd, slot)));
            }
            overflow.push(hwnd);
        }
    }
    // Forget windows that are tiled again (or gone), so they re-center if they overflow later.
    CENTERED.set(overflow);
    move_windows(&moves);
}

/// Reorders windows so each keeps the slot it currently covers, so an existing
/// arrangement survives restarting win-tile. Used once, on startup.
pub fn order_by_position(windows: &[HWND]) -> Vec<HWND> {
    let mut ordered = Vec::with_capacity(windows.len());
    for (monitor, group) in group_by_monitor(windows) {
        let Some(area) = work_area(monitor) else {
            ordered.extend(group);
            continue;
        };
        let slots = layout::layout(area, group.len(), settings::gap());
        let rects: Vec<Rect> = group
            .iter()
            .map(|&hwnd| window::visible_rect(hwnd).unwrap_or_default())
            .collect();
        ordered.extend(
            layout::match_slots(&slots, &rects)
                .into_iter()
                .map(|i| group[i]),
        );
    }
    ordered
}

pub fn same_monitor(a: HWND, b: HWND) -> bool {
    monitor_of(a) == monitor_of(b)
}

pub fn monitor_of(hwnd: HWND) -> HMONITOR {
    unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) }
}

/// If `a` and `b` are on different monitors, moves each onto the other's spot. Retiling
/// lays out each monitor using the windows currently on it, so a swap across monitors
/// has to move them across first.
pub fn exchange_monitors(a: HWND, b: HWND) {
    if same_monitor(a, b) {
        return;
    }
    if let (Some(rect_a), Some(rect_b)) = (window::visible_rect(a), window::visible_rect(b)) {
        move_windows(&[(a, outer_rect(a, rect_b)), (b, outer_rect(b, rect_a))]);
    }
}

/// Moves all windows in one batch so they reflow together instead of one at a time.
/// If the batch fails (e.g. it includes an admin window we may not move), Windows
/// discards the whole batch, so fall back to moving them one by one.
fn move_windows(moves: &[(HWND, Rect)]) {
    let flags = SWP_NOZORDER | SWP_NOACTIVATE;
    let batched = unsafe {
        BeginDeferWindowPos(moves.len() as i32).and_then(|mut batch| {
            for &(hwnd, r) in moves {
                batch = DeferWindowPos(batch, hwnd, None, r.x, r.y, r.w, r.h, flags)?;
            }
            EndDeferWindowPos(batch)
        })
    };
    if batched.is_err() {
        for &(hwnd, r) in moves {
            let _ = unsafe { SetWindowPos(hwnd, None, r.x, r.y, r.w, r.h, flags) };
        }
    }
}

/// Centers a window on its monitor (keeping its size), e.g. when it starts floating.
pub fn center(hwnd: HWND) {
    if let Some(area) = work_area(monitor_of(hwnd))
        && let Some(slot) = centered_slot(hwnd, area)
    {
        move_windows(&[(hwnd, outer_rect(hwnd, slot))]);
    }
}

/// Where a window should go to sit centered on the work area, keeping its size but never
/// larger than the area.
fn centered_slot(hwnd: HWND, area: Rect) -> Option<Rect> {
    let mut outer = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut outer) }.ok()?;
    let b = invisible_borders(hwnd).unwrap_or_default();
    let gap = settings::gap();
    let w = (outer.right - outer.left - b.left - b.right).min(area.w - 2 * gap);
    let h = (outer.bottom - outer.top - b.top - b.bottom).min(area.h - 2 * gap);
    Some(Rect {
        x: area.x + (area.w - w) / 2,
        y: area.y + (area.h - h) / 2,
        w,
        h,
    })
}

/// Splits the list by monitor, keeping each monitor's windows in list order.
fn group_by_monitor(windows: &[HWND]) -> Vec<(HMONITOR, Vec<HWND>)> {
    let mut groups: Vec<(HMONITOR, Vec<HWND>)> = Vec::new();
    for &hwnd in windows {
        let monitor = monitor_of(hwnd);
        match groups.iter_mut().find(|(m, _)| *m == monitor) {
            Some((_, group)) => group.push(hwnd),
            None => groups.push((monitor, vec![hwnd])),
        }
    }
    groups
}

/// The monitor's area minus the taskbar.
fn work_area(monitor: HMONITOR) -> Option<Rect> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return None;
    }
    let r = info.rcWork;
    Some(Rect {
        x: r.left,
        y: r.top,
        w: r.right - r.left,
        h: r.bottom - r.top,
    })
}

/// The full window rectangle to request so the window's *visible* frame fills `slot`.
/// Also un-maximizes the window, since a maximized window ignores size changes.
fn outer_rect(hwnd: HWND, slot: Rect) -> Rect {
    if unsafe { IsZoomed(hwnd) }.as_bool() {
        let _ = unsafe { ShowWindow(hwnd, SW_RESTORE) };
    }
    let b = invisible_borders(hwnd).unwrap_or_default();
    Rect {
        x: slot.x - b.left,
        y: slot.y - b.top,
        w: slot.w + b.left + b.right,
        h: slot.h + b.top + b.bottom,
    }
}

/// Windows 10/11 windows have invisible resize borders (~7px) outside their visible
/// frame. `GetWindowRect` includes them; DWM's extended frame bounds don't. The
/// difference on each side is the border width.
fn invisible_borders(hwnd: HWND) -> Option<RECT> {
    let mut outer = RECT::default();
    let mut visible = RECT::default();
    unsafe {
        GetWindowRect(hwnd, &mut outer).ok()?;
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut visible as *mut RECT as *mut _,
            size_of::<RECT>() as u32,
        )
        .ok()?;
    }
    Some(RECT {
        left: visible.left - outer.left,
        top: visible.top - outer.top,
        right: outer.right - visible.right,
        bottom: outer.bottom - visible.bottom,
    })
}
