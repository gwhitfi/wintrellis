//! The only place windows get hidden or unhidden. Every hidden window is written to a
//! file *before* it's hidden, so if WinTrellis crashes or is killed, the next start (or
//! the panic hook) can bring it back. Losing a hidden window is the worst failure mode.

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    IsWindow, IsWindowVisible, SW_HIDE, SW_SHOWNA, ShowWindow,
};

thread_local! {
    static HIDDEN: RefCell<Vec<HWND>> = const { RefCell::new(Vec::new()) };
}

fn record_path() -> PathBuf {
    std::env::temp_dir().join("wintrellis-hidden.txt")
}

pub fn hide(hwnd: HWND) {
    HIDDEN.with_borrow_mut(|hidden| {
        if !hidden.contains(&hwnd) {
            hidden.push(hwnd);
        }
        save(hidden);
    });
    let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
}

pub fn show(hwnd: HWND) {
    // SW_SHOWNA: show without stealing focus.
    let _ = unsafe { ShowWindow(hwnd, SW_SHOWNA) };
    HIDDEN.with_borrow_mut(|hidden| {
        hidden.retain(|&h| h != hwnd);
        save(hidden);
    });
}

pub fn is_hidden(hwnd: HWND) -> bool {
    HIDDEN.with_borrow(|hidden| hidden.contains(&hwnd))
}

/// Stops tracking a window that no longer exists, without trying to show it.
pub fn forget(hwnd: HWND) {
    HIDDEN.with_borrow_mut(|hidden| {
        if hidden.contains(&hwnd) {
            hidden.retain(|&h| h != hwnd);
            save(hidden);
        }
    });
}

/// Unhides everything we hid. Called on a clean exit.
pub fn restore_all() {
    HIDDEN.take();
    recover();
}

/// Unhides every window listed in the record file and deletes it. Called on startup (to
/// recover from a crash), from the panic hook, and by `restore_all`. Reads the file
/// rather than memory, so it works even if our in-memory state is mid-update.
pub fn recover() {
    let Ok(contents) = fs::read_to_string(record_path()) else {
        return;
    };
    for line in contents.lines() {
        let Ok(value) = line.trim().parse::<usize>() else {
            continue;
        };
        let hwnd = HWND(value as *mut _);
        // Skip handles that are gone, or already visible.
        unsafe {
            if IsWindow(Some(hwnd)).as_bool() && !IsWindowVisible(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_SHOWNA);
            }
        }
    }
    let _ = fs::remove_file(record_path());
}

/// Installs a panic hook that unhides windows before the panic message is printed.
pub fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // The default hook prints to stderr, which is gone once there's no console.
        log!("{info}");
        recover();
        default_hook(info);
    }));
}

fn save(hidden: &[HWND]) {
    let contents: String = hidden
        .iter()
        .map(|hwnd| format!("{}\n", hwnd.0 as usize))
        .collect();
    if let Err(err) = fs::write(record_path(), contents) {
        log!("failed to record hidden windows: {err}");
    }
}
