//! The launcher: a numbered popup menu of the `[[launcher]]` favorites from the
//! settings file, shown in the middle of the focused monitor. Picking one opens it.

use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, GetForegroundWindow, MF_STRING,
    SW_SHOWNORMAL, SetForegroundWindow, TPM_CENTERALIGN, TPM_RETURNCMD, TPM_VCENTERALIGN,
    TrackPopupMenu,
};
use windows::core::{HSTRING, w};

use crate::{display, settings, tiler};

/// Shows the menu and waits for a choice (Esc or clicking elsewhere cancels).
pub fn show() {
    let favorites = settings::launcher();
    if favorites.is_empty() {
        log!("the launcher has no apps: add [[launcher]] entries to the settings file");
        return;
    }
    let owner = display::listener();
    let at = menu_position();
    let chosen = unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        for (i, favorite) in favorites.iter().enumerate() {
            // Item IDs are the index + 1, because 0 means "nothing chosen".
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                i + 1,
                &HSTRING::from(label(i, &favorite.name)),
            );
        }
        // The menu only gets keyboard input (and closes on outside clicks) if its
        // owner window is in the foreground.
        let _ = SetForegroundWindow(owner);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_CENTERALIGN | TPM_VCENTERALIGN,
            at.x,
            at.y,
            None,
            owner,
            None,
        );
        let _ = DestroyMenu(menu);
        chosen.0 as usize
    };
    if let Some(favorite) = chosen.checked_sub(1).and_then(|i| favorites.get(i)) {
        open(&favorite.open);
    }
}

/// "&1  Firefox": `&` makes the next character the item's shortcut key while the menu
/// is open. The 10th app gets 0; later ones have no shortcut. A literal `&` is `&&`.
fn label(index: usize, name: &str) -> String {
    let name = name.replace('&', "&&");
    match index {
        0..=8 => format!("&{}  {name}", index + 1),
        9 => format!("&0  {name}"),
        _ => format!("     {name}"),
    }
}

/// The middle of the focused monitor's work area, or the mouse position if that fails.
fn menu_position() -> POINT {
    let monitor = tiler::monitor_of(unsafe { GetForegroundWindow() });
    if let Some(area) = tiler::work_area(monitor) {
        return POINT {
            x: area.x + area.w / 2,
            y: area.y + area.h / 2,
        };
    }
    let mut cursor = POINT::default();
    let _ = unsafe { GetCursorPos(&mut cursor) };
    cursor
}

/// Opens it the way the Run dialog (Win+R) does, so programs, paths, links like
/// `steam://` and `shell:AppsFolder\...` Store apps all work.
fn open(target: &str) {
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(target),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    // Values above 32 mean success; lower ones are error codes.
    if result.0 as usize <= 32 {
        log!("couldn't open {target} (error {})", result.0 as usize);
    }
}
