use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GA_ROOT, GW_OWNER, GWL_EXSTYLE, GWL_STYLE, GetAncestor, GetClassNameW, GetWindow,
    GetWindowLongPtrW, GetWindowTextW, IsIconic, IsWindowVisible, MINMAXINFO, SMTO_ABORTIFHUNG,
    SendMessageTimeoutW, WM_GETMINMAXINFO, WS_EX_TOOLWINDOW, WS_THICKFRAME,
};
use windows::core::BOOL;

use crate::layout::Rect;

/// Returns every visible top-level window, in Z-order (frontmost first).
pub fn visible_windows() -> Vec<HWND> {
    let mut found: Vec<HWND> = Vec::new();
    let lparam = LPARAM(&mut found as *mut Vec<HWND> as isize);
    let _ = unsafe { EnumWindows(Some(collect_visible), lparam) };
    found
}

/// Called by `EnumWindows` once per top-level window. `lparam` carries our `Vec`.
unsafe extern "system" fn collect_visible(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let found = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    if unsafe { IsWindowVisible(hwnd) }.as_bool() {
        found.push(hwnd);
    }
    true.into() // keep enumerating
}

/// Why a visible window should not be tiled, or `None` if it should be.
pub fn skip_reason(hwnd: HWND) -> Option<&'static str> {
    if unsafe { GetAncestor(hwnd, GA_ROOT) } != hwnd {
        return Some("child window");
    }
    if unsafe { IsIconic(hwnd) }.as_bool() {
        return Some("minimized");
    }
    if is_cloaked(hwnd) {
        return Some("cloaked");
    }
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
        return Some("tool window");
    }
    // Tiling means resizing, so a window without a resizable frame can't be tiled.
    // This also catches borderless popups (e.g. the taskbar's "PopupHost").
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
    if style & WS_THICKFRAME.0 == 0 {
        return Some("not resizable");
    }
    if has_owner(hwnd) {
        return Some("owned");
    }
    if title(hwnd).is_empty() {
        return Some("no title");
    }
    match class_name(hwnd).as_str() {
        "Progman" => Some("desktop"),
        "IME" => Some("input method helper"),
        _ => None,
    }
}

/// Cloaked windows are hidden by Windows itself (suspended UWP apps, other virtual desktops).
fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked: u32 = 0;
    let result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut _,
            size_of::<u32>() as u32,
        )
    };
    result.is_ok() && cloaked != 0
}

/// Owned windows are dialogs/popups that belong to another window.
fn has_owner(hwnd: HWND) -> bool {
    unsafe { GetWindow(hwnd, GW_OWNER) }.is_ok_and(|owner| !owner.is_invalid())
}

/// The window's visible frame on screen (without the invisible resize borders).
pub fn visible_rect(hwnd: HWND) -> Option<Rect> {
    let mut r = RECT::default();
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut RECT as *mut _,
            size_of::<RECT>() as u32,
        )
    }
    .ok()?;
    Some(Rect {
        x: r.left,
        y: r.top,
        w: r.right - r.left,
        h: r.bottom - r.top,
    })
}

/// The smallest size the window's app allows, as (width, height) of the whole window
/// (invisible borders included), or (0, 0) if it sets none. Asked with
/// WM_GETMINMAXINFO, which Windows passes on to other programs' windows; an app that's
/// hung is given up on after 50 ms.
pub fn min_size(hwnd: HWND) -> (i32, i32) {
    let mut info = MINMAXINFO::default();
    let answered = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_GETMINMAXINFO,
            WPARAM(0),
            LPARAM(&mut info as *mut MINMAXINFO as isize),
            SMTO_ABORTIFHUNG,
            50,
            None,
        )
    };
    if answered.0 == 0 {
        return (0, 0);
    }
    (info.ptMinTrackSize.x, info.ptMinTrackSize.y)
}

pub fn title(hwnd: HWND) -> String {
    let mut buffer = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    utf16_to_string(&buffer, len)
}

pub fn class_name(hwnd: HWND) -> String {
    let mut buffer = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buffer) };
    utf16_to_string(&buffer, len)
}

fn utf16_to_string(buffer: &[u16], len: i32) -> String {
    String::from_utf16_lossy(&buffer[..len.max(0) as usize])
}
