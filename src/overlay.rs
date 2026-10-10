//! A focus border we draw ourselves: an invisible, click-through window placed just
//! around the focused window, painted as a colored frame. Unlike the native border
//! color, apps that draw their own frame (Windows Terminal, Discord) can't overwrite it.

use std::cell::Cell;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CombineRgn, CreateRoundRectRgn, CreateSolidBrush, DeleteObject, EndPaint, FillRect,
    FillRgn, InvalidateRect, PAINTSTRUCT, RGN_DIFF,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, HWND_TOP, LWA_COLORKEY, RegisterClassW,
    SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetLayeredWindowAttributes, SetWindowPos, ShowWindow,
    WM_ERASEBKGND, WM_PAINT, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

use crate::{settings, window};

/// The middle of the overlay is painted this color, which Windows then treats as fully
/// transparent (and click-through). Picked to be a color nobody would choose for a border.
const SEE_THROUGH: COLORREF = COLORREF(0x0001_0001);

thread_local! {
    static OVERLAY: Cell<Option<HWND>> = const { Cell::new(None) };

    /// Corner radius of the window the border is around, in pixels (0 = square).
    static RADIUS: Cell<i32> = const { Cell::new(0) };
}

/// Creates the (hidden) overlay window. Call once, on the message-loop thread.
pub fn create() -> windows::core::Result<()> {
    let instance = unsafe { GetModuleHandleW(None) }?;
    let class = WNDCLASSW {
        lpfnWndProc: Some(on_message),
        hInstance: instance.into(),
        lpszClassName: w!("win-tile-focus-border"),
        ..Default::default()
    };
    let hwnd = unsafe {
        RegisterClassW(&class);
        // Layered + transparent: clicks pass through to the window underneath.
        // Tool window + no-activate: never in the taskbar or Alt+Tab, never takes focus.
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class.lpszClassName,
            w!("win-tile border"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )?
    };
    unsafe { SetLayeredWindowAttributes(hwnd, SEE_THROUGH, 0, LWA_COLORKEY) }?;
    OVERLAY.set(Some(hwnd));
    Ok(())
}

/// Places the border around `target` and shows it, just above `target` in the stacking
/// order (the focused window is at the top, so this puts the border on top of it).
pub fn show_around(target: HWND) {
    let (Some(overlay), Some(r)) = (OVERLAY.get(), window::visible_rect(target)) else {
        return;
    };
    RADIUS.set(settings::border_radius().unwrap_or_else(|| matching_radius(target)));
    // Drawn just outside the window's visible edge, in the gap.
    let width = settings::border_width();
    unsafe {
        let _ = SetWindowPos(
            overlay,
            Some(HWND_TOP),
            r.x - width,
            r.y - width,
            r.w + 2 * width,
            r.h + 2 * width,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        // Repaint, in case only the color changed.
        let _ = InvalidateRect(Some(overlay), None, false);
    }
}

/// The radius Windows uses for the window's corners, given the `corners` setting. Windows
/// scales its corner radius with display scaling (8px at 100% is 12px at 150%).
fn matching_radius(target: HWND) -> i32 {
    let at_100_percent = match settings::corners() {
        1 => 0, // square
        3 => 4, // small
        _ => 8, // round, or Windows' default (round)
    };
    let dpi = unsafe { GetDpiForWindow(target) }.max(96) as i32;
    at_100_percent * dpi / 96
}

pub fn hide() {
    if let Some(overlay) = OVERLAY.get() {
        let _ = unsafe { ShowWindow(overlay, SW_HIDE) };
    }
}

unsafe extern "system" fn on_message(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        // We paint every pixel ourselves; skipping the background erase avoids flicker.
        WM_ERASEBKGND => LRESULT(1),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Paints everything see-through, then a frame `border_width` pixels thick: the area
/// between an outer rounded rectangle and an inner one that hugs the window's corners.
fn paint(hwnd: HWND) {
    let width = settings::border_width();
    let radius = RADIUS.get();
    // The outer edge curves around the inner one; a square window gets a square frame.
    let outer_radius = if radius > 0 { radius + width } else { 0 };
    let mut ps = PAINTSTRUCT::default();
    unsafe {
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut all = RECT::default();
        let _ = GetClientRect(hwnd, &mut all);
        let clear = CreateSolidBrush(SEE_THROUGH);
        FillRect(hdc, &all, clear);

        // Rounded regions leave off their last row and column, hence the +1s.
        // The size arguments are the corner's diameter, not its radius.
        let frame = CreateRoundRectRgn(
            all.left,
            all.top,
            all.right + 1,
            all.bottom + 1,
            2 * outer_radius,
            2 * outer_radius,
        );
        let hole = CreateRoundRectRgn(
            all.left + width,
            all.top + width,
            all.right - width + 1,
            all.bottom - width + 1,
            2 * radius,
            2 * radius,
        );
        CombineRgn(Some(frame), Some(frame), Some(hole), RGN_DIFF);
        let border = CreateSolidBrush(COLORREF(settings::border_color()));
        let _ = FillRgn(hdc, frame, border);

        let _ = DeleteObject(border.into());
        let _ = DeleteObject(clear.into());
        let _ = DeleteObject(frame.into());
        let _ = DeleteObject(hole.into());
        let _ = EndPaint(hwnd, &ps);
    }
}
