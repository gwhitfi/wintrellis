use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE, EVENT_OBJECT_SHOW,
    EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND,
    EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZESTART,
    GetForegroundWindow, GetShellWindow, OBJID_WINDOW, PostMessageW, SetForegroundWindow,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_APP,
};

use crate::layout::{self, Direction, Splits};
use crate::{bar, border, display, hidden, style, tiler, window};

/// Posted to the message loop when the managed list changed and windows need moving.
pub const WM_RETILE: u32 = WM_APP + 2;

#[derive(Clone, Copy)]
struct Entry {
    hwnd: HWND,
    /// Which workspace of its monitor the window belongs to.
    workspace: u8,
    /// Floating windows stay managed (workspaces, focus, border) but aren't tiled.
    floating: bool,
    /// Set when the window floats only because it didn't fit: how many windows were
    /// tiled on its monitor then (itself included). Once there are fewer, it's retried.
    too_big_with: Option<usize>,
}

thread_local! {
    /// Windows we manage, in the order we started managing them. Only touched on the
    /// main thread (WinEvent callbacks are delivered through its message loop).
    static MANAGED: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };

    /// The workspace each monitor is showing. Monitors not listed show workspace 1.
    static ACTIVE: RefCell<Vec<(HMONITOR, u8)>> = const { RefCell::new(Vec::new()) };

    /// Each workspace's dividing lines, once resized. Workspaces not listed are even.
    static SPLITS: RefCell<Vec<((HMONITOR, u8), Splits)>> = const { RefCell::new(Vec::new()) };

    /// The monitor of the managed window that last had focus, or the one the desktop was
    /// clicked on. Workspace hotkeys act on this monitor. Tracked ourselves because after
    /// switching to an empty workspace, Windows may hand focus to a window on another
    /// monitor.
    static FOCUSED_MONITOR: Cell<Option<HMONITOR>> = const { Cell::new(None) };

    /// True while the desktop is getting focus because we focused it (an empty workspace),
    /// not because it was clicked. Its focus event then leaves `FOCUSED_MONITOR` alone.
    static FOCUSING_DESKTOP: Cell<bool> = const { Cell::new(false) };

    /// The managed window that has focus, and the monitor it was on.
    static FOCUSED_WINDOW: Cell<Option<(HWND, HMONITOR)>> = const { Cell::new(None) };

    /// The managed window that had focus before that one, its monitor, and when focus
    /// left it. Windows may report focus jumping away from a closing window before it
    /// reports the window closing.
    static PREVIOUS_FOCUS: Cell<Option<(HWND, HMONITOR, u32)>> = const { Cell::new(None) };

    /// Set when the focused window closed (or minimized) and left its monitor's workspace
    /// empty: that monitor, and when (milliseconds since Windows started). Windows then
    /// hands focus to some window, often on another monitor; that one automatic jump is
    /// ignored, so workspace hotkeys stay on the empty monitor.
    static KEPT: Cell<Option<(HMONITOR, u32)>> = const { Cell::new(None) };

    /// A monitor the next new window should be moved to, and when that was asked (see
    /// `put_next_window_on`).
    static PLACE_NEXT: Cell<Option<(HMONITOR, u32)>> = const { Cell::new(None) };

    /// The window being dragged/resized with the mouse, and the monitor it started on.
    static DRAG_START: Cell<Option<(HWND, HMONITOR)>> = const { Cell::new(None) };

    /// True while a WM_RETILE is queued, so a burst of changes causes only one retile.
    static RETILE_PENDING: Cell<bool> = const { Cell::new(false) };

    /// Windows added since the last retile, still to be given a slot that fits them.
    static NEW_WINDOWS: RefCell<Vec<HWND>> = const { RefCell::new(Vec::new()) };
}

/// Handles WM_RETILE: moves every managed window into its slot.
pub fn retile() {
    RETILE_PENDING.set(false);
    for hwnd in NEW_WINDOWS.take() {
        place(hwnd);
    }
    retry_too_big();
    let too_big = tiler::retile(&tiled(), shown_splits);
    for hwnd in too_big {
        float_too_big(hwnd);
    }
    border::reposition();
    bar::refresh();
}

/// A window that can't shrink to fit its slot (it has a minimum size) floats instead,
/// centered, and the others are retiled without it. It's tiled again by itself once
/// there's more room (see `retry_too_big`), or with Win+Shift+Space.
fn float_too_big(hwnd: HWND) {
    let tiled_with_it = tiled_on(tiler::monitor_of(hwnd)).len();
    MANAGED.with_borrow_mut(|managed| {
        if let Some(entry) = managed.iter_mut().find(|e| e.hwnd == hwnd) {
            entry.floating = true;
            entry.too_big_with = Some(tiled_with_it);
        }
    });
    log!(
        "{hwnd:?} {} is bigger than its slot (minimum size), floating it",
        window::title(hwnd)
    );
    tiler::center(hwnd);
    request_retile();
}

/// Gives a new window the last slot on its monitor where every window, it included,
/// fits (going by the apps' minimum sizes). Usually that's the end, its slot as
/// appended; an app with a large minimum size (Spotify, Discord) moves up to a bigger
/// slot instead and the windows after it shift down one. If no slot works, it stays at
/// the end and the retile floats it.
fn place(hwnd: HWND) {
    if !is_managed(hwnd) || is_floating(hwnd) {
        return; // closed again, or floating
    }
    let monitor = tiler::monitor_of(hwnd);
    let others: Vec<HWND> = tiled_on(monitor)
        .into_iter()
        .filter(|&h| h != hwnd)
        .collect();
    if others.len() >= layout::MAX_TILED {
        return; // no slot left for it anyway: it floats as a 7th+ window
    }
    let splits = shown_splits(monitor);
    let fits_at = |k: usize| {
        let mut windows = others.clone();
        windows.insert(k, hwnd);
        tiler::fits(&windows, splits)
    };
    // From the end backwards, so the existing arrangement moves as little as possible.
    let Some(k) = (0..=others.len()).rev().find(|&k| fits_at(k)) else {
        return;
    };
    if k == others.len() {
        return; // the end fits: keep it there
    }
    MANAGED.with_borrow_mut(|managed| {
        let Some(from) = managed.iter().position(|e| e.hwnd == hwnd) else {
            return;
        };
        let entry = managed.remove(from);
        // Just before the window that held slot `k`, so it takes over that slot.
        let to = managed
            .iter()
            .position(|e| e.hwnd == others[k])
            .unwrap_or(managed.len());
        managed.insert(to, entry);
    });
    log!(
        "{hwnd:?} {} placed in slot {k}, where it fits",
        window::title(hwnd)
    );
}

/// Puts windows that float only because they didn't fit back into the layout, once
/// fewer windows are tiled on their monitor than when they were floated (one closed or
/// moved away) and every app's minimum size says they'd now fit. Checked before
/// anything moves, so a window that still doesn't fit stays put instead of flickering.
fn retry_too_big() {
    let waiting: Vec<(HWND, usize)> = on_screen()
        .filter_map(|e| Some((e.hwnd, e.too_big_with?)))
        .collect();
    for (hwnd, tiled_then) in waiting {
        let monitor = tiler::monitor_of(hwnd);
        // In list order, so each window is checked against the slot it would get.
        let with_it: Vec<HWND> = on_screen()
            .filter(|e| (!e.floating || e.hwnd == hwnd) && tiler::monitor_of(e.hwnd) == monitor)
            .map(|e| e.hwnd)
            .collect();
        if with_it.len() < tiled_then
            && with_it.len() <= layout::MAX_TILED
            && tiler::fits(&with_it, shown_splits(monitor))
        {
            MANAGED.with_borrow_mut(|managed| {
                if let Some(entry) = managed.iter_mut().find(|e| e.hwnd == hwnd) {
                    entry.floating = false;
                    entry.too_big_with = None;
                }
            });
            log!("{hwnd:?} {} fits again, tiling it", window::title(hwnd));
        }
    }
}

/// The tiled windows on `monitor`'s active workspace, in slot order.
fn tiled_on(monitor: HMONITOR) -> Vec<HWND> {
    tiled()
        .into_iter()
        .filter(|&hwnd| tiler::monitor_of(hwnd) == monitor)
        .collect()
}

/// The dividing lines of the workspace `monitor` is showing.
fn shown_splits(monitor: HMONITOR) -> Splits {
    let workspace = (monitor, active_workspace(monitor));
    SPLITS.with_borrow(|all| {
        all.iter()
            .find(|&&(w, _)| w == workspace)
            .map_or(Splits::EVEN, |&(_, splits)| splits)
    })
}

fn set_shown_splits(monitor: HMONITOR, splits: Splits) {
    let workspace = (monitor, active_workspace(monitor));
    SPLITS.with_borrow_mut(|all| match all.iter_mut().find(|(w, _)| *w == workspace) {
        Some((_, current)) => *current = splits,
        None => all.push((workspace, splits)),
    });
}

/// Moves a border of the focused tiled window a step in `dir` (see `layout::resize`),
/// which resizes its neighbors too. Skipped if a window can't shrink that far (it has
/// a minimum size), so resizing never knocks a window out of the layout.
pub fn resize(dir: Direction) {
    let hwnd = unsafe { GetForegroundWindow() };
    let monitor = tiler::monitor_of(hwnd);
    let here: Vec<HWND> = tiled()
        .into_iter()
        .filter(|&other| tiler::monitor_of(other) == monitor)
        .collect();
    // Not tiled (floating, unmanaged, or past the 6th window): nothing to resize.
    let Some(slot) = here.iter().position(|&other| other == hwnd) else {
        return;
    };
    let before = shown_splits(monitor);
    let Some(after) = layout::resize(before, here.len(), slot, dir) else {
        return;
    };
    // Check first, so a step that doesn't fit doesn't move anything at all.
    if !tiler::fits(&here, after) {
        return;
    }
    set_shown_splits(monitor, after);
    // Some apps enforce a minimum size without reporting it: undo the step afterwards.
    if !tiler::retile(&tiled(), shown_splits).is_empty() {
        log!("can't resize further: a neighbor of {hwnd:?} can't shrink any more");
        set_shown_splits(monitor, before);
        tiler::retile(&tiled(), shown_splits);
    }
    border::reposition();
}

/// Puts every dividing line of the focused monitor's workspace back in the middle.
pub fn reset_sizes() {
    let monitor = tiler::monitor_of(unsafe { GetForegroundWindow() });
    set_shown_splits(monitor, Splits::EVEN);
    request_retile();
}

pub fn is_managed(hwnd: HWND) -> bool {
    position(hwnd).is_some()
}

fn position(hwnd: HWND) -> Option<usize> {
    MANAGED.with_borrow(|managed| managed.iter().position(|e| e.hwnd == hwnd))
}

/// How many workspaces each monitor has (hotkeys 1–5).
pub const WORKSPACES: usize = 5;

/// What the workspace bar shows for `monitor`: the workspace it's showing, and for each
/// workspace (index 0 = workspace 1) whether any managed window belongs to it there.
/// Hidden windows still count: Windows keeps their position, so their monitor is known.
pub fn workspace_summary(monitor: HMONITOR) -> (u8, [bool; WORKSPACES]) {
    let mut occupied = [false; WORKSPACES];
    MANAGED.with_borrow(|managed| {
        for e in managed {
            if tiler::monitor_of(e.hwnd) == monitor
                && let Some(slot) = usize::from(e.workspace)
                    .checked_sub(1)
                    .and_then(|i| occupied.get_mut(i))
            {
                *slot = true;
            }
        }
    });
    (active_workspace(monitor), occupied)
}

fn active_workspace(monitor: HMONITOR) -> u8 {
    ACTIVE.with_borrow(|active| {
        active
            .iter()
            .find(|&&(m, _)| m == monitor)
            .map_or(1, |&(_, workspace)| workspace)
    })
}

/// Shows workspace `n` on the focused monitor (see `switch_workspace_on`).
pub fn switch_workspace(n: u8) {
    let monitor = FOCUSED_MONITOR
        .get()
        .unwrap_or_else(|| tiler::monitor_of(unsafe { GetForegroundWindow() }));
    switch_workspace_on(monitor, n);
}

/// Shows workspace `n` on `monitor`: hides that monitor's current windows, shows the
/// ones belonging to `n`, and focuses the first of them. Workspace hotkeys act on
/// `monitor` from now on.
pub fn switch_workspace_on(monitor: HMONITOR, n: u8) {
    FOCUSED_MONITOR.set(Some(monitor));
    let current = active_workspace(monitor);
    if n == current {
        return;
    }
    ACTIVE.with_borrow_mut(|active| {
        active.retain(|&(m, _)| m != monitor);
        active.push((monitor, n));
    });
    let on_monitor: Vec<Entry> = MANAGED
        .with_borrow(|managed| managed.clone())
        .into_iter()
        .filter(|e| tiler::monitor_of(e.hwnd) == monitor)
        .collect();
    for e in on_monitor.iter().filter(|e| e.workspace == current) {
        hidden::hide(e.hwnd);
    }
    let showing: Vec<HWND> = on_monitor
        .iter()
        .filter(|e| e.workspace == n)
        .map(|e| e.hwnd)
        .collect();
    for &hwnd in &showing {
        hidden::show(hwnd);
    }
    log!("workspace {n} ({} windows)", showing.len());
    // An empty workspace focuses the desktop, so focus doesn't jump to another monitor.
    match showing.first() {
        Some(&hwnd) => {
            let _ = unsafe { SetForegroundWindow(hwnd) };
        }
        None => focus_desktop(),
    }
    request_retile();
}

/// How long `put_next_window_on` waits for the window to appear.
const PLACE_NEXT_MS: u32 = 5000;

/// The next window that appears within `PLACE_NEXT_MS` is moved to `monitor` before
/// it's tiled, so it's tiled there. For apps that open where they like, ignoring where
/// they're asked to (Settings reopens wherever it was last).
pub fn put_next_window_on(monitor: HMONITOR) {
    PLACE_NEXT.set(Some((monitor, unsafe { GetTickCount() })));
}

/// How long after keeping a monitor (see `KEPT`) a focus jump to another monitor is
/// taken as Windows' automatic one, not the user's.
const KEEP_MS: u32 = 500;

/// Focuses the desktop without changing which monitor workspace hotkeys act on (the
/// desktop spans every monitor, so its focus event can't tell which one is meant).
fn focus_desktop() {
    let desktop = unsafe { GetShellWindow() };
    if unsafe { GetForegroundWindow() } == desktop {
        return; // no focus change, so no event to skip
    }
    if unsafe { SetForegroundWindow(desktop) }.as_bool() {
        FOCUSING_DESKTOP.set(true);
    }
}

/// Sends the focused window to workspace `n` on its monitor. It's hidden until that
/// workspace is shown, where it takes the next free slot. Focus moves to another window
/// still on screen on that monitor (or the desktop if none is left).
pub fn move_to_workspace(n: u8) {
    let hwnd = unsafe { GetForegroundWindow() };
    let monitor = tiler::monitor_of(hwnd);
    if active_workspace(monitor) == n {
        return;
    }
    let Some(i) = position(hwnd) else {
        return;
    };
    MANAGED.with_borrow_mut(|managed| {
        let mut entry = managed.remove(i);
        entry.workspace = n;
        managed.push(entry);
    });
    hidden::hide(hwnd);
    log!("moved {hwnd:?} to workspace {n}");
    match managed()
        .into_iter()
        .find(|&h| tiler::monitor_of(h) == monitor)
    {
        Some(next) => {
            let _ = unsafe { SetForegroundWindow(next) };
        }
        None => focus_desktop(),
    }
    request_retile();
}

/// Monitors or the taskbar changed. Windows on a monitor that disappeared get moved
/// to another monitor by Windows; they join whatever workspace that monitor is showing.
/// Then everything is retiled to the new screen sizes.
pub fn display_changed() {
    MANAGED.with_borrow_mut(|managed| {
        for e in managed.iter_mut().filter(|e| !hidden::is_hidden(e.hwnd)) {
            e.workspace = active_workspace(tiler::monitor_of(e.hwnd));
        }
    });
    log!("display changed, retiling");
    request_retile();
}

/// Puts back the styling we changed on every managed window (on exit).
pub fn release_all() {
    for e in MANAGED.take() {
        style::release(e.hwnd);
    }
}

/// Re-applies styling to every managed window, e.g. after settings were reloaded.
pub fn restyle_all() {
    for e in MANAGED.with_borrow(|managed| managed.clone()) {
        style::apply(e.hwnd);
    }
}

/// Swaps two managed windows' places in the list (and so their slots).
pub fn swap(a: HWND, b: HWND) {
    if is_floating(a) || is_floating(b) {
        return;
    }
    if let (Some(i), Some(j)) = (position(a), position(b)) {
        MANAGED.with_borrow_mut(|managed| managed.swap(i, j));
        tiler::exchange_monitors(a, b);
        request_retile();
    }
}

/// Moves `hwnd` to the front of the list, which is slot 0 on its monitor. If it's
/// already first on its monitor, moves the next window on that monitor to the front.
/// A floating (or 7th+) window joins the layout there: slot 0 is the biggest, so an
/// app with a large minimum size fits best. If it still doesn't, the retile floats it
/// again.
pub fn promote(hwnd: HWND) {
    if !is_managed(hwnd) {
        return;
    }
    let target = match tiled_on(tiler::monitor_of(hwnd)).as_slice() {
        [first, second, ..] if *first == hwnd => *second,
        _ => hwnd,
    };
    MANAGED.with_borrow_mut(|managed| {
        if let Some(i) = managed.iter().position(|e| e.hwnd == target) {
            let mut entry = managed.remove(i);
            if entry.floating {
                log!(
                    "{target:?} {} promoted into the layout",
                    window::title(target)
                );
            }
            entry.floating = false;
            entry.too_big_with = None;
            managed.insert(0, entry);
        }
    });
    request_retile();
}

/// The managed windows on each monitor's active workspace (the ones on screen), in order.
pub fn managed() -> Vec<HWND> {
    on_screen().map(|e| e.hwnd).collect()
}

/// Like `managed`, but without floating windows: the ones that get tiled.
pub fn tiled() -> Vec<HWND> {
    on_screen()
        .filter(|e| !e.floating)
        .map(|e| e.hwnd)
        .collect()
}

fn on_screen() -> impl Iterator<Item = Entry> {
    let all = MANAGED.with_borrow(|managed| managed.clone());
    all.into_iter()
        .filter(|e| e.workspace == active_workspace(tiler::monitor_of(e.hwnd)))
}

fn is_floating(hwnd: HWND) -> bool {
    MANAGED.with_borrow(|managed| managed.iter().any(|e| e.hwnd == hwnd && e.floating))
}

/// Takes the focused window out of the layout (centered, on top) or puts it back into
/// the slot it had before.
pub fn toggle_floating() {
    let hwnd = unsafe { GetForegroundWindow() };
    let floating = MANAGED.with_borrow_mut(|managed| {
        let entry = managed.iter_mut().find(|e| e.hwnd == hwnd)?;
        entry.floating = !entry.floating;
        // Your choice now, so it's no longer retried automatically.
        entry.too_big_with = None;
        Some(entry.floating)
    });
    match floating {
        Some(true) => tiler::center(hwnd),
        Some(false) => {}
        None => return, // not managed
    }
    request_retile();
}

/// Retiling happens later in the message loop, not inside the event callback, so moving
/// windows can't re-enter the callback while `MANAGED` is borrowed. The note goes to
/// the listener window (see `display.rs`): a note to the thread can be thrown away by
/// another message loop, and then `RETILE_PENDING` would stay set and nothing would
/// ever be retiled again. Before the listener exists (startup), to the thread.
fn request_retile() {
    if !RETILE_PENDING.replace(true) {
        let _ = unsafe { PostMessageW(display::listener(), WM_RETILE, WPARAM(0), LPARAM(0)) };
    }
}

/// Picks up existing windows, then listens for windows appearing and disappearing.
/// The calling thread must run a message loop.
pub fn start() -> [HWINEVENTHOOK; 5] {
    let existing: Vec<HWND> = window::visible_windows()
        .into_iter()
        .filter(|&hwnd| window::skip_reason(hwnd).is_none())
        .collect();
    // Windows lists them in focus order; keep the on-screen arrangement instead.
    for hwnd in tiler::order_by_position(&existing) {
        add(hwnd);
    }
    // They keep the slots they're already in, not the ones that fit best.
    NEW_WINDOWS.take();
    border::refresh();
    [
        // DESTROY, SHOW, HIDE are consecutive event IDs, so one hook covers all three.
        hook_events(EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE),
        // Modern (UWP) apps are shown while still cloaked, then uncloaked.
        hook_events(EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
        // Minimized windows give up their tile until restored.
        hook_events(EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
        // Focus changes, for the focus border.
        hook_events(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
        // Mouse drags and resizes, to snap windows back into the layout.
        hook_events(EVENT_SYSTEM_MOVESIZESTART, EVENT_SYSTEM_MOVESIZEEND),
    ]
}

fn hook_events(first: u32, last: u32) -> HWINEVENTHOOK {
    unsafe {
        SetWinEventHook(
            first,
            last,
            None,
            Some(on_win_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    }
}

unsafe extern "system" fn on_win_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    time: u32,
) {
    // Only care about windows themselves, not parts of them (scrollbars, carets, ...).
    if id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32 {
        return;
    }
    match event {
        EVENT_SYSTEM_FOREGROUND => {
            if is_managed(hwnd) {
                let monitor = tiler::monitor_of(hwnd);
                if !is_automatic_jump(monitor, time) {
                    FOCUSED_MONITOR.set(Some(monitor));
                }
                if let Some((old, old_monitor)) = FOCUSED_WINDOW.get()
                    && old != hwnd
                {
                    PREVIOUS_FOCUS.set(Some((old, old_monitor, time)));
                }
                FOCUSED_WINDOW.set(Some((hwnd, monitor)));
            } else if window::is_desktop(hwnd) && !FOCUSING_DESKTOP.replace(false) {
                // Clicked: the desktop covers every monitor, so use the one under the mouse.
                FOCUSED_MONITOR.set(Some(tiler::monitor_at_cursor()));
            }
        }
        EVENT_OBJECT_SHOW | EVENT_OBJECT_UNCLOAKED | EVENT_SYSTEM_MINIMIZEEND
            if window::skip_reason(hwnd).is_none() =>
        {
            add(hwnd)
        }
        // Windows we hid for another workspace stay managed.
        EVENT_OBJECT_HIDE | EVENT_OBJECT_CLOAKED | EVENT_SYSTEM_MINIMIZESTART
            if !hidden::is_hidden(hwnd) =>
        {
            remove(hwnd)
        }
        EVENT_SYSTEM_MOVESIZESTART if is_managed(hwnd) => {
            DRAG_START.set(Some((hwnd, tiler::monitor_of(hwnd))));
        }
        EVENT_SYSTEM_MOVESIZEEND if is_managed(hwnd) => dropped(hwnd),
        // A window can also be closed while hidden (e.g. from its tray icon).
        EVENT_OBJECT_DESTROY => {
            hidden::forget(hwnd);
            remove(hwnd);
        }
        _ => return,
    }
    // Focus moved, or the focused window may have just become (un)managed.
    border::refresh();
}

/// A managed window was let go after a mouse drag or resize. Retiling snaps it back into
/// the layout. If it was dropped on another monitor, it joins the workspace showing there
/// and takes the next free slot.
fn dropped(hwnd: HWND) {
    let monitor = tiler::monitor_of(hwnd);
    // It may have changed monitor; keep the focused window's record right.
    if let Some((focused, _)) = FOCUSED_WINDOW.get()
        && focused == hwnd
    {
        FOCUSED_WINDOW.set(Some((hwnd, monitor)));
    }
    if let Some((dragged, start)) = DRAG_START.take()
        && dragged == hwnd
        && start != monitor
        && let Some(i) = position(hwnd)
    {
        let workspace = active_workspace(monitor);
        MANAGED.with_borrow_mut(|managed| {
            let mut entry = managed.remove(i);
            entry.workspace = workspace;
            managed.push(entry);
        });
    }
    request_retile();
}

fn add(hwnd: HWND) {
    if !is_managed(hwnd)
        && let Some((monitor, at)) = PLACE_NEXT.take()
        && unsafe { GetTickCount() }.wrapping_sub(at) < PLACE_NEXT_MS
        && tiler::monitor_of(hwnd) != monitor
    {
        tiler::center_on(hwnd, monitor);
    }
    // New windows join the workspace their monitor is showing.
    let workspace = active_workspace(tiler::monitor_of(hwnd));
    MANAGED.with_borrow_mut(|managed| {
        if !managed.iter().any(|e| e.hwnd == hwnd) {
            managed.push(Entry {
                hwnd,
                workspace,
                floating: false,
                too_big_with: None,
            });
            style::apply(hwnd);
            NEW_WINDOWS.with_borrow_mut(|new| new.push(hwnd));
            log!(
                "+ {hwnd:?} {}  ({} managed)",
                window::title(hwnd),
                managed.len()
            );
            request_retile();
        }
    });
}

fn remove(hwnd: HWND) {
    let removed = MANAGED.with_borrow_mut(|managed| {
        let index = managed.iter().position(|e| e.hwnd == hwnd)?;
        managed.remove(index);
        style::release(hwnd);
        log!("- {hwnd:?}  ({} managed)", managed.len());
        request_retile();
        Some(())
    });
    if removed.is_some() {
        keep_monitor_if_emptied(hwnd);
    }
}

/// `hwnd` just stopped being managed (closed, minimized, hidden). If it had focus and
/// was the last window on its monitor's workspace, workspace hotkeys stay on that
/// monitor instead of following focus wherever Windows sends it next, and the desktop
/// is focused, as when switching to an empty workspace.
fn keep_monitor_if_emptied(hwnd: HWND) {
    let now = unsafe { GetTickCount() };
    // Either it has focus, or focus left it a moment ago (Windows already reported the
    // jump to another window).
    let (monitor, already_jumped) = match (FOCUSED_WINDOW.get(), PREVIOUS_FOCUS.get()) {
        (Some((focused, monitor)), _) if focused == hwnd => (monitor, false),
        (_, Some((previous, monitor, at)))
            if previous == hwnd && now.wrapping_sub(at) < KEEP_MS =>
        {
            (monitor, true)
        }
        _ => return,
    };
    if managed()
        .into_iter()
        .any(|h| tiler::monitor_of(h) == monitor)
    {
        return; // other windows are still showing there
    }
    PREVIOUS_FOCUS.set(None);
    FOCUSED_MONITOR.set(Some(monitor));
    if !already_jumped {
        // The jump is still to come: ignore it when it arrives.
        FOCUSED_WINDOW.set(None);
        KEPT.set(Some((monitor, now)));
    }
    log!("last window on its monitor closed: workspace keys stay there");
    focus_desktop();
}

/// Whether focus arriving on `monitor` at `time` (milliseconds since Windows started,
/// as WinEvents report it) is Windows' own jump right after `keep_monitor_if_emptied`,
/// which shouldn't move workspace hotkeys. Only the first focus change after keeping
/// counts.
fn is_automatic_jump(monitor: HMONITOR, time: u32) -> bool {
    match KEPT.take() {
        // `wrapping_sub`: the millisecond count wraps around every 49.7 days.
        Some((kept, at)) => kept != monitor && time.wrapping_sub(at) < KEEP_MS,
        None => false,
    }
}
