use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering};
use std::thread;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    VIRTUAL_KEY, VK_LCONTROL, VK_LSHIFT, VK_LWIN, VK_RCONTROL, VK_RSHIFT, VK_RWIN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG,
    PostThreadMessageW, SetWindowsHookExW, WH_KEYBOARD_LL, WM_APP, WM_KEYDOWN, WM_SYSKEYDOWN,
};

use crate::config::Combo;
use crate::settings;

/// Posted by the hook to the main thread, with the `Action` index in `wParam`.
/// Slow work happens there, never in the hook.
pub const WM_ACTION: u32 = WM_APP + 1;

/// Posted by the hook to the main thread for a launcher app's own hotkey, with the
/// app's index in the `[[launcher]]` list in `wParam`.
pub const WM_LAUNCH: u32 = WM_APP + 4;

/// What a hotkey does: one of WinTrellis's actions, or opening a launcher app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Action(Action),
    /// Index into the `[[launcher]]` list.
    Launch(usize),
}

/// The thread that runs actions (the main thread).
static ACTION_THREAD: AtomicU32 = AtomicU32::new(0);

/// Everything a hotkey can trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    OpenLauncher,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    SwapLeft,
    SwapRight,
    SwapUp,
    SwapDown,
    Promote,
    ResizeLeft,
    ResizeRight,
    ResizeUp,
    ResizeDown,
    ResetSizes,
    Workspace1,
    Workspace2,
    Workspace3,
    Workspace4,
    Workspace5,
    MoveToWorkspace1,
    MoveToWorkspace2,
    MoveToWorkspace3,
    MoveToWorkspace4,
    MoveToWorkspace5,
    CloseWindow,
    ToggleFloating,
    ReloadConfig,
    EditConfig,
    Quit,
}

impl Action {
    /// Every action, in declaration order, so an action survives a round trip through `wParam`.
    pub const ALL: [Action; 30] = [
        Action::OpenLauncher,
        Action::FocusLeft,
        Action::FocusRight,
        Action::FocusUp,
        Action::FocusDown,
        Action::SwapLeft,
        Action::SwapRight,
        Action::SwapUp,
        Action::SwapDown,
        Action::Promote,
        Action::ResizeLeft,
        Action::ResizeRight,
        Action::ResizeUp,
        Action::ResizeDown,
        Action::ResetSizes,
        Action::Workspace1,
        Action::Workspace2,
        Action::Workspace3,
        Action::Workspace4,
        Action::Workspace5,
        Action::MoveToWorkspace1,
        Action::MoveToWorkspace2,
        Action::MoveToWorkspace3,
        Action::MoveToWorkspace4,
        Action::MoveToWorkspace5,
        Action::CloseWindow,
        Action::ToggleFloating,
        Action::ReloadConfig,
        Action::EditConfig,
        Action::Quit,
    ];

    pub fn from_wparam(wparam: WPARAM) -> Option<Action> {
        Self::ALL.get(wparam.0).copied()
    }
}

/// True while either Win key (our SUPER modifier) is held down.
static SUPER_DOWN: AtomicBool = AtomicBool::new(false);

/// True while either Shift key is held down.
static SHIFT_DOWN: AtomicBool = AtomicBool::new(false);

/// True while either Ctrl key is held down.
static CTRL_DOWN: AtomicBool = AtomicBool::new(false);

/// The key currently held down (0 if none). A key-down for the same key again is
/// Windows' auto-repeat, which we use to stop held hotkeys from firing repeatedly.
static HELD_KEY: AtomicU16 = AtomicU16::new(0);

/// Unassigned virtual-key code. Tapping it while Win is held stops Windows from
/// treating the Win press as a lone tap, so the Start menu doesn't open on release.
const MASK_KEY: VIRTUAL_KEY = VIRTUAL_KEY(0xE8);

/// Runs the keyboard hook on its own thread, posting actions to `action_thread`.
/// Windows delivers every keystroke system-wide through the hook's thread, so keeping
/// it free of other work (retiling, window events) keeps typing lag-free everywhere.
pub fn start_hook_thread(action_thread: u32) {
    ACTION_THREAD.store(action_thread, Ordering::Relaxed);
    thread::spawn(|| {
        let _hook = install_hook().expect("failed to install keyboard hook");
        // Hook calls are delivered while this thread waits here.
        let mut msg = MSG::default();
        while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {}
    });
}

/// Installs the global low-level keyboard hook. The calling thread must run a
/// message loop, since Windows delivers hook calls through it.
fn install_hook() -> windows::core::Result<HHOOK> {
    let module = unsafe { GetModuleHandleW(None) }?;
    unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), Some(module.into()), 0) }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        // Ignore synthetic input (including our own mask key) so it can't affect key tracking.
        if info.flags.contains(LLKHF_INJECTED) {
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        }
        let message = wparam.0 as u32;
        let key_down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
        let vk = VIRTUAL_KEY(info.vkCode as u16);
        let is_repeat = if key_down {
            HELD_KEY.swap(vk.0, Ordering::Relaxed) == vk.0
        } else {
            let _ = HELD_KEY.compare_exchange(vk.0, 0, Ordering::Relaxed, Ordering::Relaxed);
            false
        };

        if vk == VK_LWIN || vk == VK_RWIN {
            // Track SUPER, but let Windows see it too.
            SUPER_DOWN.store(key_down, Ordering::Relaxed);
        } else if vk == VK_LSHIFT || vk == VK_RSHIFT {
            // Same for Shift: track it, pass it through.
            SHIFT_DOWN.store(key_down, Ordering::Relaxed);
        } else if vk == VK_LCONTROL || vk == VK_RCONTROL {
            // And Ctrl.
            CTRL_DOWN.store(key_down, Ordering::Relaxed);
        } else if SUPER_DOWN.load(Ordering::Relaxed)
            && let Some(command) = settings::command_for(Combo {
                key: vk.0,
                shift: SHIFT_DOWN.load(Ordering::Relaxed),
                ctrl: CTRL_DOWN.load(Ordering::Relaxed),
            })
        {
            if key_down && !is_repeat {
                send_mask_key();
                let (message, index) = match command {
                    Command::Action(action) => (WM_ACTION, action as usize),
                    Command::Launch(app) => (WM_LAUNCH, app),
                };
                let thread = ACTION_THREAD.load(Ordering::Relaxed);
                let _ = unsafe { PostThreadMessageW(thread, message, WPARAM(index), LPARAM(0)) };
            }
            // Swallow both the press and the release so no app sees the key.
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn send_mask_key() {
    let inputs = [
        key_input(MASK_KEY, KEYBD_EVENT_FLAGS(0)),
        key_input(MASK_KEY, KEYEVENTF_KEYUP),
    ];
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
}

fn key_input(vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                dwFlags: flags,
                ..Default::default()
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_round_trips_through_wparam() {
        for action in Action::ALL {
            assert_eq!(Action::from_wparam(WPARAM(action as usize)), Some(action));
        }
    }
}
