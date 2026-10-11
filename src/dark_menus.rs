//! Dark popup menus (tray and launcher). Windows has no public API for this: the
//! undocumented uxtheme.dll functions below, looked up by number, are what Explorer,
//! Notepad++ and many other apps use. If they're ever missing, menus just stay light.

use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{PCSTR, w};

/// uxtheme.dll #135, `SetPreferredAppMode`. 2 = always dark.
const SET_PREFERRED_APP_MODE: usize = 135;
const FORCE_DARK: i32 = 2;
/// uxtheme.dll #136, `FlushMenuThemes`: applies the mode to menus right away.
const FLUSH_MENU_THEMES: usize = 136;

/// Call once at startup, before any menu is shown.
pub fn enable() {
    unsafe {
        let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) else {
            return;
        };
        // A number instead of a name in GetProcAddress means "the function with
        // this ordinal".
        let Some(set_mode) = GetProcAddress(uxtheme, PCSTR(SET_PREFERRED_APP_MODE as _)) else {
            log!("dark menus aren't available on this Windows version");
            return;
        };
        let set_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(set_mode);
        set_mode(FORCE_DARK);
        if let Some(flush) = GetProcAddress(uxtheme, PCSTR(FLUSH_MENU_THEMES as _)) {
            let flush: unsafe extern "system" fn() = std::mem::transmute(flush);
            flush();
        }
    }
}
