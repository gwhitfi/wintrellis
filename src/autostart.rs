//! "Start with Windows": a value under the current user's `Run` registry key, which
//! Windows reads at every sign-in and launches. It shows in Task Manager's Startup apps
//! page and needs no admin rights. (An elevated build would need a Task Scheduler task
//! instead: Windows skips elevated programs listed here.)

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{PCWSTR, w};

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE_NAME: PCWSTR = w!("WinTrellis");

/// What Windows runs at sign-in: this .exe's path, quoted in case it has spaces.
fn command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", exe.display()))
}

/// True if Windows will start *this* .exe at sign-in. A value pointing at another copy
/// (e.g. a dev build) counts as off, so turning it on again fixes the path.
pub fn is_enabled() -> bool {
    let Some(expected) = command() else {
        return false;
    };
    let mut buf = [0u16; 1024];
    let mut size = size_of_val(&buf) as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE_NAME,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if result != ERROR_SUCCESS {
        return false;
    }
    // `size` is in bytes and includes the terminating null.
    let len = (size as usize / 2).saturating_sub(1);
    String::from_utf16_lossy(&buf[..len]).eq_ignore_ascii_case(&expected)
}

pub fn enable() {
    let Some(command) = command() else {
        log!("couldn't find our own .exe path for start with Windows");
        return;
    };
    let data: Vec<u16> = command.encode_utf16().chain([0]).collect();
    let result = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE_NAME,
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            size_of_val(data.as_slice()) as u32,
        )
    };
    if result != ERROR_SUCCESS {
        log!("couldn't turn on start with Windows: {result:?}");
    }
}

pub fn disable() {
    let result = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE_NAME) };
    // Already gone is fine.
    if result != ERROR_SUCCESS && result != ERROR_FILE_NOT_FOUND {
        log!("couldn't turn off start with Windows: {result:?}");
    }
}
