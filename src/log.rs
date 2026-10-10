//! Log file at `%APPDATA%\win-tile\win-tile.log`. Once win-tile runs without a console,
//! this is where status messages and settings errors go.

use std::fmt;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::settings;

static FILE: Mutex<Option<File>> = Mutex::new(None);

/// Writes a timestamped line to the log file (and the console, if there is one).
/// Same arguments as `println!`.
macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write(format_args!($($arg)*)) };
}

pub fn path() -> PathBuf {
    settings::path().with_file_name("win-tile.log")
}

/// Starts a fresh log for this run. The previous run's log is kept as
/// `win-tile.previous.log`, so after a crash you can still read what happened.
pub fn init() {
    let path = path();
    let _ = fs::create_dir_all(path.parent().unwrap_or(&path));
    let _ = fs::rename(&path, path.with_file_name("win-tile.previous.log"));
    match File::create(&path) {
        Ok(file) => *FILE.lock().unwrap_or_else(|e| e.into_inner()) = Some(file),
        Err(err) => eprintln!("couldn't create {}: {err}", path.display()),
    }
}

/// Use the `log!` macro instead of calling this directly.
pub fn write(args: fmt::Arguments) {
    let t = unsafe { GetLocalTime() };
    let line = format!(
        "{:02}:{:02}:{:02}.{:03} {args}\n",
        t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    );
    print!("{line}");
    // `File` is unbuffered: each line reaches the OS immediately, so it survives a crash.
    if let Some(file) = FILE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = file.write_all(line.as_bytes());
    }
}
