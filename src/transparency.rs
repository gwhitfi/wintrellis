//! Makes windows of the programs listed in `[transparency]` see-through while they
//! don't have focus. Windows can only blend a whole window if it's "layered", so we
//! add that flag while it's see-through and take it off again when it's focused.

use std::cell::RefCell;
use std::path::Path;

use windows::Win32::Foundation::{COLORREF, CloseHandle, HWND};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, GetWindowThreadProcessId, LWA_ALPHA,
    SetLayeredWindowAttributes, SetWindowLongPtrW, WS_EX_LAYERED,
};
use windows::core::PWSTR;

use crate::settings;

thread_local! {
    /// Windows we made layered (so we only ever undo our own change).
    static MADE_LAYERED: RefCell<Vec<HWND>> = const { RefCell::new(Vec::new()) };
}

/// See-through if it's a listed program and not focused; solid otherwise.
pub fn update(hwnd: HWND, focused: bool) {
    if !focused && is_listed(hwnd) {
        make_see_through(hwnd);
    } else {
        restore(hwnd);
    }
}

/// Undoes our change, if we made one.
pub fn restore(hwnd: HWND) {
    let ours = MADE_LAYERED.with_borrow_mut(|made| {
        let ours = made.contains(&hwnd);
        made.retain(|&h| h != hwnd);
        ours
    });
    if ours {
        let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
        let _ =
            unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style & !(WS_EX_LAYERED.0 as isize)) };
    }
}

fn make_see_through(hwnd: HWND) {
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    let already_layered = ex_style & WS_EX_LAYERED.0 as isize != 0;
    let ours = MADE_LAYERED.with_borrow(|made| made.contains(&hwnd));
    if already_layered && !ours {
        // The app does its own layering; changing it would break how it draws.
        return;
    }
    if !ours {
        unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style | WS_EX_LAYERED.0 as isize) };
        MADE_LAYERED.with_borrow_mut(|made| made.push(hwnd));
    }
    let alpha = settings::unfocused_alpha();
    let _ = unsafe { SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA) };
}

fn is_listed(hwnd: HWND) -> bool {
    let apps = settings::transparent_apps();
    !apps.is_empty() && program_name(hwnd).is_some_and(|name| apps.contains(&name))
}

/// The lowercase file name of the program that owns the window, e.g. "discord.exe".
fn program_name(hwnd: HWND) -> Option<String> {
    let mut process_id = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    let process =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.ok()?;
    let mut buffer = [0u16; 1024];
    let mut len = buffer.len() as u32;
    let result = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
    };
    let _ = unsafe { CloseHandle(process) };
    result.ok()?;
    let path = String::from_utf16_lossy(&buffer[..len as usize]);
    let name = Path::new(&path).file_name()?.to_str()?;
    Some(name.to_ascii_lowercase())
}
