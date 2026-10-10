use std::cell::Cell;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_COLOR_DEFAULT, DwmSetWindowAttribute,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, EVENT_OBJECT_LOCATIONCHANGE, GetForegroundWindow, GetWindowThreadProcessId,
    OBJID_WINDOW, WINEVENT_OUTOFCONTEXT,
};

use crate::{overlay, settings, tracker, transparency};

thread_local! {
    /// The managed window that has focus (and so the border), if any.
    static BORDERED: Cell<Option<HWND>> = const { Cell::new(None) };

    /// Listens for the focused window moving, so the overlay can follow it.
    static MOVE_HOOK: Cell<Option<HWINEVENTHOOK>> = const { Cell::new(None) };
}

/// Puts the border on the focused window if it's managed, and takes it off the previous one.
pub fn refresh() {
    let foreground = unsafe { GetForegroundWindow() };
    let target = tracker::is_managed(foreground).then_some(foreground);
    let previous = BORDERED.replace(target);
    if previous == target {
        return;
    }
    if let Some(old) = previous {
        set_border_color(old, DWMWA_COLOR_DEFAULT);
        if tracker::is_managed(old) {
            transparency::update(old, false);
        }
    }
    if let Some(new) = target {
        transparency::update(new, true);
    }
    follow_moves(target);
    match target {
        Some(new) if settings::overlay_border() => overlay::show_around(new),
        Some(new) => {
            set_border_color(new, settings::border_color());
            overlay::hide();
        }
        None => overlay::hide(),
    }
}

/// Moves the overlay border back around the focused window, e.g. after a retile.
pub fn reposition() {
    if let Some(focused) = BORDERED.get()
        && settings::overlay_border()
    {
        overlay::show_around(focused);
    }
}

/// Redraws the border, e.g. after the border settings changed.
pub fn repaint() {
    clear();
    refresh();
}

/// Takes the border off (on exit).
pub fn clear() {
    if let Some(hwnd) = BORDERED.take() {
        set_border_color(hwnd, DWMWA_COLOR_DEFAULT);
    }
    follow_moves(None);
    overlay::hide();
}

/// Watches `target` for moves (e.g. mouse drags) so the overlay can follow. Window-move
/// events fire constantly system-wide, so we only listen to the one program that owns
/// the focused window.
fn follow_moves(target: Option<HWND>) {
    if let Some(hook) = MOVE_HOOK.take() {
        let _ = unsafe { UnhookWinEvent(hook) };
    }
    let Some(target) = target.filter(|_| settings::overlay_border()) else {
        return;
    };
    let mut process_id = 0;
    unsafe { GetWindowThreadProcessId(target, Some(&mut process_id)) };
    let hook = unsafe {
        SetWinEventHook(
            EVENT_OBJECT_LOCATIONCHANGE,
            EVENT_OBJECT_LOCATIONCHANGE,
            None,
            Some(on_move),
            process_id,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    MOVE_HOOK.set(Some(hook));
}

unsafe extern "system" fn on_move(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    // That program also reports carets, scrollbars, etc. moving; only the window counts.
    let is_window = id_object == OBJID_WINDOW.0 && id_child == CHILDID_SELF as i32;
    if is_window && BORDERED.get() == Some(hwnd) {
        overlay::show_around(hwnd);
    }
}

/// Sets the color of Windows 11's own (native) border for this window.
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
