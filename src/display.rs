//! Listens for display changes (monitors plugged/unplugged, resolution or scaling
//! changes) and work-area changes (taskbar moved or auto-hidden). Windows only
//! broadcasts these to top-level windows, so we create an invisible one.
//! It also hears about logoff/shutdown, which is likewise only sent to windows.

use std::sync::atomic::{AtomicUsize, Ordering};

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
use crate::{shutdown, tracker, tray};

/// The listener window, once created. Popup menus (tray, launcher) use it as their owner.
static LISTENER: AtomicUsize = AtomicUsize::new(0);

pub fn listener() -> HWND {
    HWND(LISTENER.load(Ordering::Relaxed) as _)
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
    unsafe {
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
    }
    .inspect(|hwnd| LISTENER.store(hwnd.0 as usize, Ordering::Relaxed))
}

unsafe extern "system" fn on_message(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let work_area_changed = msg == WM_SETTINGCHANGE && wparam.0 as u32 == SPI_SETWORKAREA.0;
    if msg == WM_DISPLAYCHANGE || work_area_changed {
        tracker::display_changed();
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
