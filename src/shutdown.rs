use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
use windows::core::BOOL;

/// The thread running the message loop, which must do the cleanup.
static MAIN_THREAD: AtomicU32 = AtomicU32::new(0);

/// Makes Ctrl+C (or closing the console) end the message loop instead of killing the
/// process outright, so `main` gets to clean up. Call from the message-loop thread.
pub fn install() -> windows::core::Result<()> {
    MAIN_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::Relaxed);
    unsafe { SetConsoleCtrlHandler(Some(on_console_event), true) }
}

/// Windows calls this on its own thread. Our state lives on the main thread, so all we
/// do here is ask the main thread to quit.
unsafe extern "system" fn on_console_event(_event: u32) -> BOOL {
    let main = MAIN_THREAD.load(Ordering::Relaxed);
    let _ = unsafe { PostThreadMessageW(main, WM_QUIT, WPARAM(0), LPARAM(0)) };
    // When the console window is closed, Windows kills the process as soon as this
    // returns. Waiting gives `main` time to clean up; it exits the process when done.
    thread::sleep(Duration::from_secs(5));
    true.into()
}
