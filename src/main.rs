// Release builds run in the background with no console window; dev builds keep the
// console so output is visible live. (The log file gets everything either way.)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Declared first so the `log!` macro is usable in every module below.
#[macro_use]
mod log;
mod autostart;
mod border;
mod config;
mod dark_menus;
mod display;
mod focus;
mod hidden;
mod icons;
mod keyboard;
mod launcher;
mod launcher_draw;
mod layout;
mod overlay;
mod search;
mod settings;
mod shutdown;
mod snap;
mod style;
mod tiler;
mod tracker;
mod transparency;
mod tray;
mod window;

use std::process::Command;

use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PostQuitMessage, TranslateMessage,
};

use keyboard::{Action, WM_ACTION};
use layout::Direction;

fn main() {
    log::init();
    // Bring back anything a previous crashed run left hidden, before tracking windows.
    hidden::recover();
    hidden::install_panic_hook();
    settings::load();
    // The shell functions behind launcher icons and `shell:` links need COM on this
    // thread. Single-threaded mode, since this thread runs a message loop.
    if let Err(err) = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok() {
        log!("couldn't initialize COM: {err}");
    }
    overlay::create().unwrap();
    launcher::create().unwrap();
    let _events = tracker::start();
    keyboard::start_hook_thread(unsafe { GetCurrentThreadId() });
    shutdown::install().unwrap();
    let display_listener = display::create_listener().unwrap();
    dark_menus::enable();
    tray::add(display_listener);
    snap::disable();
    log!("WinTrellis running. SUPER+Enter opens a terminal. SUPER+Esc or Ctrl+C quits.");

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        match msg.message {
            WM_ACTION => {
                if let Some(action) = Action::from_wparam(msg.wParam) {
                    run(action);
                }
            }
            tracker::WM_RETILE => tracker::retile(),
            // Messages addressed to a window (display listener, launcher) go to its
            // handler. TranslateMessage turns key presses into WM_CHAR (typed text).
            _ => unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            },
        }
    }

    // GetMessageW returned false: we got WM_QUIT (SUPER+Esc or Ctrl+C).
    shutdown::cleanup();
}

fn run(action: Action) {
    match action {
        Action::LaunchTerminal => launch_terminal(),
        Action::OpenLauncher => launcher::show(),
        Action::FocusLeft => focus::focus(Direction::Left),
        Action::FocusRight => focus::focus(Direction::Right),
        Action::FocusUp => focus::focus(Direction::Up),
        Action::FocusDown => focus::focus(Direction::Down),
        Action::SwapLeft => focus::swap(Direction::Left),
        Action::SwapRight => focus::swap(Direction::Right),
        Action::SwapUp => focus::swap(Direction::Up),
        Action::SwapDown => focus::swap(Direction::Down),
        Action::Promote => focus::promote(),
        Action::Workspace1 => tracker::switch_workspace(1),
        Action::Workspace2 => tracker::switch_workspace(2),
        Action::Workspace3 => tracker::switch_workspace(3),
        Action::Workspace4 => tracker::switch_workspace(4),
        Action::Workspace5 => tracker::switch_workspace(5),
        Action::MoveToWorkspace1 => tracker::move_to_workspace(1),
        Action::MoveToWorkspace2 => tracker::move_to_workspace(2),
        Action::MoveToWorkspace3 => tracker::move_to_workspace(3),
        Action::MoveToWorkspace4 => tracker::move_to_workspace(4),
        Action::MoveToWorkspace5 => tracker::move_to_workspace(5),
        Action::CloseWindow => focus::close(),
        Action::ToggleFloating => tracker::toggle_floating(),
        Action::ReloadConfig => {
            settings::load();
            tracker::restyle_all();
            border::repaint();
            tracker::retile();
        }
        Action::EditConfig => edit_config(),
        // Ends the message loop the same way Ctrl+C does, so cleanup runs.
        Action::Quit => unsafe { PostQuitMessage(0) },
    }
}

/// Opens the settings file in Notepad specifically, not the default editor for .toml
/// files, which might be an IDE that opens it inside some unrelated project.
fn edit_config() {
    let path = settings::path();
    if let Err(err) = Command::new("notepad.exe").arg(&path).spawn() {
        log!("failed to open {} in Notepad: {err}", path.display());
    }
}

fn launch_terminal() {
    let terminal = settings::terminal();
    if let Err(err) = Command::new(&terminal).spawn() {
        log!("failed to launch {terminal}: {err}");
    }
}
