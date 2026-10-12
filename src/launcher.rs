//! The launcher: Win+T opens a small search window in the middle of the focused
//! monitor, listing the `[[launcher]]` favorites from the settings file. Typing
//! filters the list and Enter opens the top match. Esc or clicking elsewhere closes it.

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush, DeleteDC,
    DeleteObject, EndPaint, HMONITOR, InvalidateRect, PAINTSTRUCT, SRCCOPY, SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_UP,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateCaret, CreateWindowExW, DefWindowProcW, DestroyCaret, GetClientRect, GetForegroundWindow,
    HWND_TOPMOST, IDC_ARROW, IsWindowVisible, LoadCursorW, RegisterClassW, SW_HIDE, SW_SHOWNORMAL,
    SWP_SHOWWINDOW, SetCaretPos, SetForegroundWindow, SetWindowPos, ShowCaret, ShowWindow,
    WA_INACTIVE, WM_ACTIVATE, WM_CHAR, WM_ERASEBKGND, WM_KEYDOWN, WM_KILLFOCUS, WM_PAINT,
    WM_SETFOCUS, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{HSTRING, w};

use crate::config::Favorite;
use crate::launcher_draw::{self, BACKGROUND, PADDING, ROW_HEIGHT, SEARCH_HEIGHT, View, WIDTH};
use crate::{search, settings, style, tiler};

/// What the launcher shows while it's open.
#[derive(Default)]
struct State {
    favorites: Vec<Favorite>,
    query: String,
    /// Indexes into `favorites` of the apps matching `query`, best first.
    matches: Vec<usize>,
    /// Index into `matches` of the highlighted app.
    selected: usize,
}

impl State {
    fn new(favorites: Vec<Favorite>) -> State {
        let mut state = State {
            favorites,
            ..State::default()
        };
        state.refilter();
        state
    }

    /// Recomputes the matches after the query changed, highlighting the best one.
    fn refilter(&mut self) {
        let names: Vec<&str> = self.favorites.iter().map(|f| f.name.as_str()).collect();
        self.matches = search::filter(&names, &self.query);
        self.selected = 0;
    }

    fn view(&self) -> View<'_> {
        View {
            query: &self.query,
            rows: self.matches.iter().map(|&i| &self.favorites[i]).collect(),
            selected: self.selected,
        }
    }
}

thread_local! {
    static WINDOW: Cell<Option<HWND>> = const { Cell::new(None) };

    static STATE: RefCell<State> = RefCell::new(State::default());

    /// The window that had focus before the launcher opened, to give focus back to on Esc.
    static PREVIOUS: Cell<Option<HWND>> = const { Cell::new(None) };
}

/// Creates the (hidden) launcher window. Call once, on the message-loop thread.
pub fn create() -> windows::core::Result<()> {
    let instance = unsafe { GetModuleHandleW(None) }?;
    let class = WNDCLASSW {
        lpfnWndProc: Some(on_message),
        hInstance: instance.into(),
        lpszClassName: w!("WinTrellis-launcher"),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }?,
        // Windows fills the window with this before it's painted.
        hbrBackground: unsafe { CreateSolidBrush(BACKGROUND) },
        ..Default::default()
    };
    let hwnd = unsafe {
        RegisterClassW(&class);
        // Popup: no title bar or frame. Tool window: not in the taskbar or Alt+Tab.
        // Topmost: above everything else while it's open.
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class.lpszClassName,
            w!("WinTrellis launcher"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )?
    };
    // Windows 11 rounds the corners and draws a thin outline and a shadow, smoothly.
    style::set(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND.0);
    WINDOW.set(Some(hwnd));
    Ok(())
}

/// Opens the launcher on the focused monitor and gives it keyboard focus. If it's
/// already open, closes it instead, so Win+T toggles it.
pub fn show() {
    let Some(hwnd) = WINDOW.get() else {
        return;
    };
    if unsafe { IsWindowVisible(hwnd) }.as_bool() {
        close(true);
        return;
    }
    let favorites = settings::launcher();
    if favorites.is_empty() {
        log!("the launcher has no apps: add [[launcher]] entries to the settings file");
        return;
    }
    let previous = unsafe { GetForegroundWindow() };
    PREVIOUS.set(Some(previous));
    let monitor = tiler::monitor_of(previous);
    let Some(area) = tiler::work_area(monitor) else {
        return;
    };
    // Tall enough for every app, so the search box doesn't move as the list filters.
    let rows = favorites.len() as i32;
    STATE.set(State::new(favorites));
    let scale = scaler(monitor);
    let (width, height) = (
        scale(WIDTH),
        scale(2 * PADDING + SEARCH_HEIGHT + rows * ROW_HEIGHT),
    );
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            area.x + (area.w - width) / 2,
            area.y + (area.h - height) / 2,
            width,
            height,
            SWP_SHOWWINDOW,
        );
        // Repaint even if the size didn't change: the contents did.
        let _ = InvalidateRect(Some(hwnd), None, false);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Hides the launcher. With `restore_focus`, focus goes back to the window that had it
/// (on Esc). Not when an app was opened (it takes focus itself) or the user clicked
/// somewhere else (focus is already there).
fn close(restore_focus: bool) {
    let Some(hwnd) = WINDOW.get() else {
        return;
    };
    let previous = PREVIOUS.take();
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
        if restore_focus && let Some(previous) = previous {
            let _ = SetForegroundWindow(previous);
        }
    }
}

/// Converts sizes at 100% scaling to pixels on `monitor` (150% scaling: 40 -> 60).
fn scaler(monitor: HMONITOR) -> impl Fn(i32) -> i32 {
    let (mut dpi, mut _dpi_y) = (96, 96);
    let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut _dpi_y) };
    move |px| px * dpi as i32 / 96
}

unsafe extern "system" fn on_message(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        // `paint` covers every pixel, so skip Windows' background fill (avoids flicker).
        WM_ERASEBKGND => LRESULT(1),
        // Lost focus (clicked elsewhere, Alt+Tab, ...): close. The low word of wParam
        // says whether the window was activated or deactivated.
        WM_ACTIVATE if wparam.0 as u32 & 0xFFFF == WA_INACTIVE => {
            close(false);
            LRESULT(0)
        }
        WM_KEYDOWN if on_key(VIRTUAL_KEY(wparam.0 as u16)) => LRESULT(0),
        // A typed character (Windows has already applied Shift, keyboard layout, ...).
        WM_CHAR => {
            on_char(wparam.0 as u32);
            LRESULT(0)
        }
        // The blinking text cursor: Windows' caret, shown while the launcher has focus.
        WM_SETFOCUS => {
            let (width, height) =
                launcher_draw::caret_size(unsafe { GetDpiForWindow(hwnd) } as i32);
            unsafe {
                let _ = CreateCaret(hwnd, None, width, height);
                let _ = ShowCaret(Some(hwnd));
            }
            LRESULT(0)
        }
        WM_KILLFOCUS => {
            let _ = unsafe { DestroyCaret() };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Keys that aren't characters. Returns false for keys left to Windows.
fn on_key(key: VIRTUAL_KEY) -> bool {
    match key {
        VK_ESCAPE => close(true),
        VK_RETURN => launch_selected(),
        VK_UP => change(|state| state.selected = state.selected.saturating_sub(1)),
        VK_DOWN => change(|state| {
            state.selected = (state.selected + 1).min(state.matches.len().saturating_sub(1));
        }),
        _ => return false,
    }
    true
}

/// Edits the search text. Enter and Esc also arrive here as characters, after
/// `on_key` has already handled them, so control characters are ignored.
fn on_char(c: u32) {
    const BACKSPACE: u32 = 0x08;
    // What Ctrl+Backspace types.
    const DELETE_WORD: u32 = 0x7F;
    match c {
        BACKSPACE => change(|state| {
            state.query.pop();
            state.refilter();
        }),
        DELETE_WORD => change(|state| {
            let kept = state.query.trim_end().rfind(' ').map_or(0, |i| i + 1);
            state.query.truncate(kept);
            state.refilter();
        }),
        _ => {
            if let Some(c) = char::from_u32(c).filter(|c| !c.is_control()) {
                change(|state| {
                    state.query.push(c);
                    state.refilter();
                });
            }
        }
    }
}

/// Applies `edit` to the state and repaints.
fn change(edit: impl FnOnce(&mut State)) {
    STATE.with_borrow_mut(edit);
    if let Some(hwnd) = WINDOW.get() {
        let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
    }
}

/// Opens the highlighted app (if anything matches) and closes the launcher.
fn launch_selected() {
    let target = STATE.with_borrow(|state| {
        let &app = state.matches.get(state.selected)?;
        Some(state.favorites[app].open.clone())
    });
    let Some(target) = target else {
        return;
    };
    // Open it while the launcher still has focus: Windows only lets a new program take
    // focus if the program that started it had focus.
    open(&target);
    close(false);
}

fn paint(hwnd: HWND) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let dc = BeginPaint(hwnd, &mut ps);
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let (width, height) = (client.right, client.bottom);
        // Draw into an off-screen bitmap, then copy it to the window in one go, so a
        // half-drawn frame is never visible.
        let buffer_dc = CreateCompatibleDC(Some(dc));
        let buffer = CreateCompatibleBitmap(dc, width, height);
        let old = SelectObject(buffer_dc, buffer.into());
        let dpi = GetDpiForWindow(hwnd) as i32;
        let caret = STATE
            .with_borrow(|state| launcher_draw::draw(buffer_dc, width, height, dpi, &state.view()));
        let _ = BitBlt(dc, 0, 0, width, height, Some(buffer_dc), 0, 0, SRCCOPY);
        // BeginPaint hid the caret, and EndPaint shows it again at its new spot.
        let _ = SetCaretPos(caret.x, caret.y);
        SelectObject(buffer_dc, old);
        let _ = DeleteObject(buffer.into());
        let _ = DeleteDC(buffer_dc);
        let _ = EndPaint(hwnd, &ps);
    }
}

/// Opens launcher app number `index` directly (its own hotkey), without the window.
pub fn launch(index: usize) {
    if let Some(app) = settings::launcher().get(index) {
        open(&app.open);
    }
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
