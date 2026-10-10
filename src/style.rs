//! How managed windows look: Windows 11 per-window settings we change while a window is
//! managed, and put back when it isn't (minimized, closed, or win-tile quits).

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWMWA_TRANSITIONS_FORCEDISABLED, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DEFAULT,
    DWMWINDOWATTRIBUTE, DwmSetWindowAttribute,
};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
use windows::core::BOOL;

use crate::{settings, transparency};

/// Styles a window that just became managed (or after settings were reloaded).
pub fn apply(hwnd: HWND) {
    // No show/hide/restore animations: workspace switches are instant instead of fading.
    set(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, BOOL::from(true).0);
    set(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, settings::corners());
    let focused = unsafe { GetForegroundWindow() } == hwnd;
    transparency::update(hwnd, focused);
}

/// Puts back everything `apply` changed.
pub fn release(hwnd: HWND) {
    set(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, BOOL::from(false).0);
    set(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DEFAULT.0);
    transparency::restore(hwnd);
}

/// All of these attributes take a 4-byte value.
fn set(hwnd: HWND, attribute: DWMWINDOWATTRIBUTE, value: i32) {
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            attribute,
            &value as *const i32 as *const _,
            size_of::<i32>() as u32,
        )
    };
}
