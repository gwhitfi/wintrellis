use std::cell::Cell;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_COLOR_DEFAULT, DwmSetWindowAttribute,
};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

use crate::{settings, tracker};

thread_local! {
    /// The window whose border we last colored.
    static BORDERED: Cell<Option<HWND>> = const { Cell::new(None) };
}

/// Colors the focused window's border if it's managed, and resets the previously colored one.
pub fn refresh() {
    let foreground = unsafe { GetForegroundWindow() };
    let target = tracker::is_managed(foreground).then_some(foreground);
    let previous = BORDERED.replace(target);
    if previous == target {
        return;
    }
    if let Some(old) = previous {
        set_border_color(old, DWMWA_COLOR_DEFAULT);
    }
    if let Some(new) = target {
        set_border_color(new, settings::border_color());
    }
}

/// Recolors the focused window's border, e.g. after the color setting changed.
pub fn repaint() {
    clear();
    refresh();
}

/// Puts the colored border back to normal (on exit).
pub fn clear() {
    if let Some(hwnd) = BORDERED.take() {
        set_border_color(hwnd, DWMWA_COLOR_DEFAULT);
    }
}

/// Uses Windows 11's own window border, so there's no overlay to keep in sync.
fn set_border_color(hwnd: HWND, color: u32) {
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &color as *const u32 as *const _,
            size_of::<u32>() as u32,
        )
    };
}
