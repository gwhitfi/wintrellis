//! Listens for display changes (monitors plugged/unplugged, resolution or scaling
//! changes) and work-area changes (taskbar moved or auto-hidden). Windows only
//! broadcasts these to top-level windows, so we create an invisible one.

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassW, SPI_SETWORKAREA, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_DISPLAYCHANGE, WM_SETTINGCHANGE, WNDCLASSW,
};
use windows::core::w;

use crate::tracker;

/// Creates the hidden listener window. It's never shown. Must be called on the
/// message-loop thread, since that's where its messages are delivered.
pub fn create_listener() -> windows::core::Result<HWND> {
    let instance = unsafe { GetModuleHandleW(None) }?;
    let class = WNDCLASSW {
        lpfnWndProc: Some(on_message),
        hInstance: instance.into(),
        lpszClassName: w!("win-tile-display-listener"),
        ..Default::default()
    };
    unsafe {
        RegisterClassW(&class);
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class.lpszClassName,
            w!("win-tile"),
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
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
