use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, PostMessageW, SetForegroundWindow, WM_CLOSE,
};

use crate::layout::{self, Direction};
use crate::{tracker, window};

/// Moves keyboard focus from the focused window to the nearest managed window in `dir`.
pub fn focus(dir: Direction) {
    let current = unsafe { GetForegroundWindow() };
    if let Some(target) = neighbor_window(current, dir, tracker::managed())
        && !unsafe { SetForegroundWindow(target) }.as_bool()
    {
        eprintln!("Windows refused to focus {target:?}");
    }
}

/// Swaps the focused window with its neighbor in `dir`. Focus stays on the moved window.
pub fn swap(dir: Direction) {
    let current = unsafe { GetForegroundWindow() };
    if !tracker::is_managed(current) {
        return;
    }
    if let Some(target) = neighbor_window(current, dir, tracker::tiled()) {
        tracker::swap(current, target);
    }
}

/// Moves the focused window into the big left pane of its monitor. If it's already
/// there, the next window on that monitor takes the pane instead, so pressing it
/// twice swaps back. Focus stays where it is.
pub fn promote() {
    let current = unsafe { GetForegroundWindow() };
    tracker::promote(current);
}

/// Asks the focused window to close, like clicking its X (so it can still ask to save).
/// Only managed windows, so this can't close the desktop or taskbar by accident.
pub fn close() {
    let current = unsafe { GetForegroundWindow() };
    if tracker::is_managed(current) {
        let _ = unsafe { PostMessageW(Some(current), WM_CLOSE, WPARAM(0), LPARAM(0)) };
    }
}

/// The nearest of `candidates` in `dir` from `current`, by what's on screen.
fn neighbor_window(current: HWND, dir: Direction, candidates: Vec<HWND>) -> Option<HWND> {
    let from = window::visible_rect(current)?;
    let others: Vec<_> = candidates
        .into_iter()
        .filter(|&hwnd| hwnd != current)
        .filter_map(|hwnd| Some((hwnd, window::visible_rect(hwnd)?)))
        .collect();
    let rects: Vec<_> = others.iter().map(|&(_, rect)| rect).collect();
    let i = layout::neighbor(from, &rects, dir)?;
    Some(others[i].0)
}
