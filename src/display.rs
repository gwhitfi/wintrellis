//! Listens for display changes (monitors plugged/unplugged, resolution or scaling
//! changes) and work-area changes (taskbar moved or auto-hidden). Windows only
//! broadcasts these to top-level windows, so we create an invisible one.
//! It also hears about logoff/shutdown, which is likewise only sent to windows.
//!
//! WinTrellis's own "do this later" notes (`WM_RETILE`, `WM_VOLUME`) are posted to this
//! window too, not to the thread: a note to a window survives while some other code
//! (a menu, `ShellExecuteW`) runs its own message loop on our thread, which throws
//! notes addressed to the thread away.

use std::cell::Cell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, PostThreadMessageW, RegisterClassW, SPI_SETWORKAREA,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_DISPLAYCHANGE, WM_QUERYENDSESSION, WM_RBUTTONUP,
    WM_SETTINGCHANGE, WNDCLASSW,
};
use windows::core::w;

use crate::keyboard::WM_ACTION;
use crate::{bar, shutdown, tracker, tray, volume};

thread_local! {
    static LISTENER: Cell<Option<HWND>> = const { Cell::new(None) };
}

/// The listener window, once created: where WinTrellis posts its notes to itself.
pub fn listener() -> Option<HWND> {
    LISTENER.get()
}

/// Creates the hidden listener window. It's never shown. Must be called on the
/// message-loop thread, since that's where its messages are delivered.
pub fn create_listener() -> windows::core::Result<HWND> {
    let instance = unsafe { GetModuleHandleW(None) }?;
    let class = WNDCLASSW {
        lpfnWndProc: Some(on_message),
        hInstance: instance.into(),
        lpszClassName: w!("WinTrellis-display-listener"),
        ..Default::default()
    };
    let hwnd = unsafe {
        RegisterClassW(&class);
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class.lpszClassName,
            w!("WinTrellis"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }?;
    LISTENER.set(Some(hwnd));
    Ok(hwnd)
}

unsafe extern "system" fn on_message(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        tracker::WM_RETILE => {
            tracker::retile();
            return LRESULT(0);
        }
        volume::WM_VOLUME => {
            if wparam.0 == volume::NEW_DEVICE {
                volume::use_default_device();
            }
            bar::refresh();
            return LRESULT(0);
        }
        _ => {}
    }
    let work_area_changed = msg == WM_SETTINGCHANGE && wparam.0 as u32 == SPI_SETWORKAREA.0;
    if msg == WM_DISPLAYCHANGE || work_area_changed {
        tracker::display_changed();
        bar::rebuild();
    }
    // For tray messages, lParam says what the mouse did on the icon.
    if msg == tray::WM_TRAY
        && lparam.0 as u32 == WM_RBUTTONUP
        && let Some(action) = tray::show_menu(hwnd)
    {
        // Hand it to the main loop, so it runs exactly like the same hotkey would.
        let thread = unsafe { GetCurrentThreadId() };
        let _ =
            unsafe { PostThreadMessageW(thread, WM_ACTION, WPARAM(action as usize), LPARAM(0)) };
    }
    if msg == WM_QUERYENDSESSION {
        // Logoff or shutdown is starting. Unhide everything now, before other apps get
        // asked: a hidden app's "Save changes?" prompt could otherwise be invisible.
        // We quit even if the logoff is later cancelled.
        log!("Windows is ending the session");
        shutdown::cleanup();
        std::process::exit(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
