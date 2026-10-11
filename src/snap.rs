//! Turns off Windows' own window snapping (drag to the top to maximize, the Snap
//! Layouts bar, drag to an edge to half-snap) while WinTrellis runs, since it fights the
//! tiling. Only for this session: the saved setting isn't touched, so if WinTrellis
//! crashes, snapping comes back at the next sign-in at the latest.

use std::cell::Cell;

use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETWINARRANGING, SPI_SETWINARRANGING, SystemParametersInfoW,
};
use windows::core::BOOL;

thread_local! {
    /// Whether snapping was on before we turned it off.
    static WAS_ENABLED: Cell<bool> = const { Cell::new(false) };
}

pub fn disable() {
    let mut enabled = BOOL(0);
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETWINARRANGING,
            0,
            Some(&mut enabled as *mut BOOL as *mut _),
            Default::default(),
        )
    };
    if read.is_ok() && enabled.as_bool() {
        WAS_ENABLED.set(true);
        set(false);
    }
}

/// Puts snapping back the way it was (on exit).
pub fn restore() {
    if WAS_ENABLED.replace(false) {
        set(true);
    }
}

fn set(on: bool) {
    // The new value goes in `uiParam`. No flags: SPIF_UPDATEINIFILE would save it to
    // the Windows profile, and SPIF_SENDCHANGE makes Windows wait on every open window
    // to acknowledge the change, which can stall for seconds if an app is busy.
    let _ =
        unsafe { SystemParametersInfoW(SPI_SETWINARRANGING, on as u32, None, Default::default()) };
}
