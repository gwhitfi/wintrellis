//! The notification-area (tray) icon: the visible sign that WinTrellis is running.
//! It belongs to the hidden listener window from `display.rs`, which receives its
//! mouse events as `WM_TRAY` messages.

use std::sync::atomic::{AtomicUsize, Ordering};

use windows::Win32::Foundation::{HWND, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, GetSystemMetrics, HICON, IMAGE_ICON,
    LR_DEFAULTCOLOR, LoadImageW, MF_SEPARATOR, MF_STRING, SM_CXSMICON, SM_CYSMICON,
    SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP,
};
use windows::core::{PCWSTR, w};

use crate::keyboard::Action;

/// Sent to the listener window when the mouse does something on the tray icon.
pub const WM_TRAY: u32 = WM_APP + 3;

/// The window the icon belongs to, so `remove` can find it.
static OWNER: AtomicUsize = AtomicUsize::new(0);

/// Adds the icon to the tray. `owner` is the window that gets its `WM_TRAY` messages.
pub fn add(owner: HWND) {
    OWNER.store(owner.0 as usize, Ordering::Relaxed);
    let mut data = base(owner);
    data.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE;
    data.uCallbackMessage = WM_TRAY;
    data.hIcon = load_icon();
    let tip: Vec<u16> = "WinTrellis".encode_utf16().collect();
    data.szTip[..tip.len()].copy_from_slice(&tip);
    if !unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
        log!("couldn't add the tray icon");
    }
}

/// Removes the icon. Without this it lingers in the tray until the mouse passes over it.
pub fn remove() {
    let owner = HWND(OWNER.load(Ordering::Relaxed) as _);
    if !owner.is_invalid() {
        let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &base(owner)) };
    }
}

/// Shows the right-click menu at the mouse and waits for a choice. Returns the chosen
/// action, or `None` if the menu was dismissed.
pub fn show_menu(owner: HWND) -> Option<Action> {
    // Item IDs are the action's index + 1, because 0 means "nothing chosen".
    let id = |action: Action| action as usize + 1;
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        let _ = AppendMenuW(menu, MF_STRING, id(Action::EditConfig), w!("Edit settings"));
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            id(Action::ReloadConfig),
            w!("Reload settings"),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, id(Action::Quit), w!("Quit WinTrellis"));
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        // Documented Win32 quirk: without this the menu doesn't close when you click
        // somewhere else.
        let _ = SetForegroundWindow(owner);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            cursor.x,
            cursor.y,
            None,
            owner,
            None,
        );
        let _ = DestroyMenu(menu);
        let index = (chosen.0 as usize).checked_sub(1)?;
        Action::from_wparam(WPARAM(index))
    }
}

/// The fields that identify our icon: owner window + ID. Every call needs them.
fn base(owner: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: owner,
        uID: 1,
        ..Default::default()
    }
}

/// Our embedded app icon (resource ID 1 in `wintrellis.rc`) at the small-icon size, so
/// Windows doesn't scale down the 256 px version.
fn load_icon() -> HICON {
    let size = |metric| unsafe { GetSystemMetrics(metric) };
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        LoadImageW(
            Some(instance.into()),
            PCWSTR(1 as _),
            IMAGE_ICON,
            size(SM_CXSMICON),
            size(SM_CYSMICON),
            LR_DEFAULTCOLOR,
        )
    }
    .map(|handle| HICON(handle.0))
    .unwrap_or_default()
}
