//! The workspace bar: a small pill at the top center of each monitor with one numbered
//! bubble per workspace, then the speaker volume and a clock. Filled = the workspace has windows, outlined =
//! empty, ringed = the one showing; clicking a bubble switches to its workspace. Always
//! shown: the band it sits in is kept free of windows (see `reserved_height`). Hovering
//! the clock shows the date in a small tooltip under it, hovering the volume its level;
//! scrolling over the volume changes it, clicking opens Windows' sound output panel and
//! right-clicking Sound settings.

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Globalization::{ENUM_DATE_FORMATS_FLAGS, GetDateFormatEx};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ROUNDED_RECT, ID2D1Brush, ID2D1RenderTarget, ID2D1StrokeStyle,
};
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL,
};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VIRTUAL_KEY, VK_LCONTROL, VK_LWIN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HWND_TOPMOST, IDC_ARROW, LoadCursorW,
    RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, SetTimer, SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WM_LBUTTONDOWN,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{BOOL, PCWSTR, Result, w};

use crate::layout::{self, Rect};
use crate::{canvas, keyboard, launcher, settings, tiler, tracker, volume};

// Sizes in pixels at 100% scaling.
/// Each workspace's pill. Every cell in the bar is this tall.
const BUBBLE_W: i32 = 34;
const BUBBLE_H: i32 = 16;
/// The volume icon's cell, after the bubbles, and the icon's size.
const VOLUME_W: i32 = 20;
const ICON_SIZE: i32 = 13;
/// The clock's cell, after the volume: room for "12:59 PM".
const CLOCK_W: i32 = 54;
/// The date tooltip under the clock: its height, the space either side of the date,
/// and how far below the bar it sits.
const TIP_H: i32 = 22;
const TIP_PADDING: i32 = 10;
const TIP_GAP: i32 = 4;
// The space above the bar, inside it and below it are settings (`bar_margin`,
// `bar_padding`, `bar_gap`).
/// The current workspace's ring: its distance from the bubble, and its thickness.
const RING_GAP: i32 = 1;
const LINE: i32 = 2;
const FONT: i32 = 11;
/// How far the soft shadow around the bar reaches. The window is this much bigger than
/// the bar on every side, to make room for it.
const SHADOW: i32 = 6;

/// Which of a bar's cells (see `cells`) are the volume icon and the clock; the ones
/// before them are the workspace bubbles.
const VOLUME: usize = tracker::WORKSPACES;
const CLOCK: usize = VOLUME + 1;

/// How dark the shadow is right next to the bar (0.0 none .. 1.0 black); it fades out
/// from there.
const SHADOW_ALPHA: f32 = 0.25;
/// How solid the bar's background is: slightly see-through.
const BACKGROUND_ALPHA: f32 = 0.85;

/// "The mouse left the window", sent when asked for with `TrackMouseEvent`. The
/// `windows` crate files it under Common Controls, a big feature we don't otherwise need.
const WM_MOUSELEAVE: u32 = 0x02A3;

/// The `SetTimer` id that wakes the first bar when the minute changes, to update the
/// clock on every bar.
const CLOCK_TIMER: usize = 1;

/// One monitor's bar.
#[derive(Clone, Copy)]
struct Bar {
    monitor: HMONITOR,
    bar: HWND,
    /// Where the bar's window goes on screen: the bar plus its shadow.
    rect: Rect,
}

thread_local! {
    static BARS: RefCell<Vec<Bar>> = const { RefCell::new(Vec::new()) };

    /// The date tooltip: one window, moved under whichever clock is hovered.
    static TIP: Cell<Option<HWND>> = const { Cell::new(None) };

    /// The bar and the cell (`VOLUME` or `CLOCK`) the mouse is over, if any: the
    /// tooltip shows that cell's text.
    static HOVERED: Cell<Option<(HWND, usize)>> = const { Cell::new(None) };
}

/// Registers the window classes, then creates the bars (if the `workspace_bar`
/// setting is on). Call once, on the message-loop thread.
pub fn create() -> Result<()> {
    let instance = unsafe { GetModuleHandleW(None) }?;
    unsafe {
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(on_bar_message),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: w!("WinTrellis-workspace-bar"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(on_tip_message),
            hInstance: instance.into(),
            lpszClassName: w!("WinTrellis-bar-tip"),
            ..Default::default()
        });
    }
    // Like the focus border: never focused, and clicks pass straight through it.
    let tip = new_window(
        w!("WinTrellis-bar-tip"),
        WS_EX_TRANSPARENT,
        Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        },
    )?;
    TIP.set(Some(tip));
    build()
}

/// Throws the bars away and builds them again: monitors were added, removed or
/// rescaled, the taskbar moved, or the `workspace_bar` setting changed.
pub fn rebuild() {
    unhover();
    for b in BARS.take() {
        let _ = unsafe { DestroyWindow(b.bar) };
    }
    if let Err(err) = build() {
        log!("couldn't rebuild the workspace bar: {err}");
    }
}

/// Creates and shows a bar on every monitor, if the setting is on.
fn build() -> Result<()> {
    if !settings::workspace_bar() {
        return Ok(());
    }
    for monitor in monitors() {
        let Some(area) = tiler::work_area(monitor) else {
            continue;
        };
        let px = scaler(monitor);
        let (w, h) = size(&px);
        let spot = layout::top_center(area, w, h, px(settings::bar_margin()));
        // The window is bigger than the bar by the shadow on every side, so the bar
        // itself sits where the settings say.
        let shadow = px(SHADOW);
        let rect = Rect {
            x: spot.x - shadow,
            y: spot.y - shadow,
            w: spot.w + 2 * shadow,
            h: spot.h + 2 * shadow,
        };
        let bar = new_window(w!("WinTrellis-workspace-bar"), WINDOW_EX_STYLE(0), rect)?;
        let b = Bar { monitor, bar, rect };
        BARS.with_borrow_mut(|bars| bars.push(b));
        // Drawn before it's shown, so it appears finished.
        redraw(&b);
        let _ = unsafe { ShowWindow(bar, SW_SHOWNOACTIVATE) };
    }
    // One timer for all the bars, on the first one (destroyed with it on a rebuild).
    if let Some(first) = BARS.with_borrow(|bars| bars.first().copied()) {
        schedule_clock(first.bar);
    }
    Ok(())
}

/// Wakes `bar` just after the next minute starts, so the clock changes on time. The
/// timer is set again each time it fires: from now, not at a fixed rate, so it can't
/// drift and catches up after the computer sleeps.
fn schedule_clock(bar: HWND) {
    let now = unsafe { GetLocalTime() };
    let into_minute = u32::from(now.wSecond) * 1000 + u32::from(now.wMilliseconds);
    // A little late rather than early: a timer can fire a few milliseconds off.
    let wait = 60_000 - into_minute.min(59_999) + 20;
    unsafe { SetTimer(Some(bar), CLOCK_TIMER, wait, None) };
}

/// Redraws every bar, after workspaces or windows changed.
pub fn refresh() {
    for b in BARS.with_borrow(|bars| bars.clone()) {
        redraw(&b);
    }
    // A shown tooltip may be out of date too (e.g. the volume while scrolling).
    if let Some((bar, cell)) = HOVERED.get()
        && let Some(b) = find(bar)
    {
        tip_or_log(&b, cell);
    }
}

/// How many pixels at the top of `monitor` the bar keeps for itself (0 if it's off),
/// so windows start `bar_gap` below it. WinTrellis places windows below this band
/// (`tiler::tiling_area`), so the bar never covers a window's title bar buttons.
pub fn reserved_height(monitor: HMONITOR) -> i32 {
    if !settings::workspace_bar() {
        return 0;
    }
    let px = scaler(monitor);
    layout::bar_band(
        px(settings::bar_margin()),
        size(&px).1,
        settings::bar_gap(),
        settings::gap(),
    )
}

/// The bar's (width, height) on a monitor scaled by `scale`, without the shadow.
fn size(scale: &impl Fn(i32) -> i32) -> (i32, i32) {
    layout::bar_size(
        &widths(scale),
        scale(BUBBLE_H),
        scale(settings::bar_padding()),
    )
}

/// The width of each of the bar's cells: one per workspace, then the volume and the
/// clock.
fn widths(scale: &impl Fn(i32) -> i32) -> Vec<i32> {
    let mut widths = vec![scale(BUBBLE_W); tracker::WORKSPACES];
    widths.push(scale(VOLUME_W));
    widths.push(scale(CLOCK_W));
    widths
}

/// A hidden, never-focused, always-on-top window of `class` at `r`, layered so
/// `canvas::show` can give it see-through, smooth edges (its fully see-through pixels
/// let clicks through), plus any `extra` styles.
fn new_window(class: PCWSTR, extra: WINDOW_EX_STYLE, r: Rect) -> Result<HWND> {
    let instance = unsafe { GetModuleHandleW(None) }?;
    unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST | extra,
            class,
            w!("WinTrellis workspaces"),
            WS_POPUP,
            r.x,
            r.y,
            r.w,
            r.h,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
}

/// Every monitor, via `EnumDisplayMonitors`.
fn monitors() -> Vec<HMONITOR> {
    let mut found: Vec<HMONITOR> = Vec::new();
    let lparam = LPARAM(&mut found as *mut Vec<HMONITOR> as isize);
    let _ = unsafe { EnumDisplayMonitors(None, None, Some(collect_monitor), lparam) };
    found
}

/// Called by `EnumDisplayMonitors` once per monitor. `lparam` carries our `Vec`.
unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _dc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let found = unsafe { &mut *(lparam.0 as *mut Vec<HMONITOR>) };
    found.push(monitor);
    true.into() // keep enumerating
}

/// Converts sizes at 100% scaling to pixels on `monitor` (150% scaling: 22 -> 33).
fn scaler(monitor: HMONITOR) -> impl Fn(i32) -> i32 {
    let (mut dpi, mut _dpi_y) = (96, 96);
    let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut _dpi_y) };
    move |px| px * dpi as i32 / 96
}

unsafe extern "system" fn on_bar_message(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // No WM_PAINT: a window shown with UpdateLayeredWindow keeps the image it was given.
    match msg {
        WM_LBUTTONDOWN => clicked(hwnd, lparam),
        WM_RBUTTONUP => right_clicked(hwnd, lparam),
        WM_MOUSEWHEEL if scrolled(hwnd, wparam, lparam) => {}
        WM_MOUSEMOVE => hover(hwnd, lparam),
        // Asked for by `hover` (TrackMouseEvent): the mouse left the bar.
        WM_MOUSELEAVE => unhover(),
        WM_TIMER if wparam.0 == CLOCK_TIMER => {
            refresh();
            schedule_clock(hwnd);
        }
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

/// A click on `bar`: a bubble switches its monitor to that workspace; the volume opens
/// Windows' sound output panel (Quick Settings).
fn clicked(bar: HWND, lparam: LPARAM) {
    let Some((b, cell)) = cell_at(bar, mouse_position(lparam)) else {
        return;
    };
    if cell < tracker::WORKSPACES {
        tracker::switch_workspace_on(b.monitor, cell as u8 + 1);
    } else if cell == VOLUME {
        // Win+Ctrl+V: Quick Settings, opened on its sound output page (device picker
        // and per-app volume). Windows offers no supported way to open it directly.
        keyboard::press_shortcut(&[VK_LWIN, VK_LCONTROL, VIRTUAL_KEY(b'V'.into())]);
    }
}

/// A right-click on `bar`: the volume opens Windows' Sound settings.
fn right_clicked(bar: HWND, lparam: LPARAM) {
    if let Some((b, VOLUME)) = cell_at(bar, mouse_position(lparam)) {
        // On this bar's monitor, not wherever Settings was last. Settings ignores the
        // request in `open_on`, so its window is also moved here when it appears.
        tracker::put_next_window_on(b.monitor);
        launcher::open_on("ms-settings:sound", b.monitor);
    }
}

/// The mouse wheel turned over `bar`: over the volume, changes it 2% per notch (the
/// step Windows' own slider uses). Returns whether it was used.
fn scrolled(bar: HWND, wparam: WPARAM, lparam: LPARAM) -> bool {
    let Some(b) = find(bar) else {
        return false;
    };
    // For the wheel, lParam is a position on the screen, not in the window.
    let (x, y) = mouse_position(lparam);
    let inside = (x - b.rect.x, y - b.rect.y);
    if !matches!(cell_at(bar, inside), Some((_, VOLUME))) {
        return false;
    }
    // The high 16 bits of wParam: how far it turned, 120 per notch, positive = away
    // from you (up).
    let turned = f32::from((wparam.0 >> 16) as u16 as i16) / 120.0;
    volume::change(turned * 0.02);
    true
}

/// The bar whose window is `bar`, and which of its cells (see `cells`) `position` (in
/// the window) is in.
fn cell_at(bar: HWND, (x, y): (i32, i32)) -> Option<(Bar, usize)> {
    let b = find(bar)?;
    let cell = layout::rect_at(&cells(&scaler(b.monitor)), x, y)?;
    Some((b, cell))
}

/// The mouse moved over `bar`: shows the date tooltip when it's over the clock, and
/// hides it when it moves off.
fn hover(bar: HWND, lparam: LPARAM) {
    let over =
        cell_at(bar, mouse_position(lparam)).filter(|&(_, cell)| cell == VOLUME || cell == CLOCK);
    // Mouse moves arrive constantly; act only when the mouse moves to another cell.
    if over.map(|(_, cell)| (bar, cell)) == HOVERED.get() {
        return;
    }
    let Some((b, cell)) = over else {
        unhover();
        return;
    };
    HOVERED.set(Some((bar, cell)));
    // Windows doesn't say when the mouse leaves a window unless asked, once per visit.
    let mut track = TRACKMOUSEEVENT {
        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: bar,
        dwHoverTime: 0,
    };
    let _ = unsafe { TrackMouseEvent(&mut track) };
    tip_or_log(&b, cell);
}

/// `show_tip`, logging a failure.
fn tip_or_log(b: &Bar, cell: usize) {
    if let Err(err) = show_tip(b, cell) {
        log!("couldn't show the tooltip: {err}");
    }
}

/// Hides the tooltip.
fn unhover() {
    HOVERED.set(None);
    if let Some(tip) = TIP.get() {
        let _ = unsafe { ShowWindow(tip, SW_HIDE) };
    }
}

/// Shows the tooltip for `cell` of `b` centered under it: today's date for the clock,
/// the level for the volume.
fn show_tip(b: &Bar, cell: usize) -> Result<()> {
    let text = if cell == CLOCK {
        date_text()
    } else {
        Some(volume_text(volume::level()))
    };
    let (Some(tip), Some(text)) = (TIP.get(), text) else {
        return Ok(());
    };
    let scale = scaler(b.monitor);
    let under = cells(&scale)[cell];
    let format = canvas::text_format(scale(FONT) as f32, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
    // Never wrapped onto more lines, whatever the measurement says.
    canvas::one_line(&format)?;
    // Just wide enough for the text, plus room for the shadow, like the bar.
    let text_w = canvas::text_width(&text, &format)?.ceil() as i32;
    let shadow = scale(SHADOW);
    let w = text_w + 2 * scale(TIP_PADDING) + 2 * shadow;
    let h = scale(TIP_H) + 2 * shadow;
    let bar_bottom = b.rect.y + b.rect.h - shadow;
    let at = Rect {
        x: b.rect.x + under.x + under.w / 2 - w / 2,
        y: bar_bottom + scale(TIP_GAP) - shadow,
        w,
        h,
    };
    canvas::show(tip, at, |target| {
        draw_tip(target, &text, &format, at, shadow)
    })?;
    unsafe {
        // On top of the bar and everything else, without taking focus.
        SetWindowPos(
            tip,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
    }
}

/// The tooltip: a pill like the bar, with its shadow, and `text` in the middle.
fn draw_tip(
    target: &ID2D1RenderTarget,
    text: &str,
    format: &IDWriteTextFormat,
    at: Rect,
    shadow: i32,
) -> Result<()> {
    let colors = settings::colors();
    let body = Rect {
        x: shadow,
        y: shadow,
        w: at.w - 2 * shadow,
        h: at.h - 2 * shadow,
    };
    draw_shadow(target, body, shadow)?;
    let wide: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        let background = target.CreateSolidColorBrush(
            &canvas::color(COLORREF(colors.background), BACKGROUND_ALPHA),
            None,
        )?;
        let text_brush =
            target.CreateSolidColorBrush(&canvas::color(COLORREF(colors.text), 1.0), None)?;
        target.FillRoundedRectangle(&pill(body, 0.0), &background);
        target.DrawText(
            &wide,
            format,
            &rect_f(body),
            &text_brush,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }
    Ok(())
}

/// Today's date, short, with day and month names in the user's language: e.g.
/// "Sun 11 Oct". `None` if Windows can't format it.
fn date_text() -> Option<String> {
    let mut buffer = [0u16; 64];
    let len = unsafe {
        GetDateFormatEx(
            PCWSTR::null(), // the user's own language and region
            ENUM_DATE_FORMATS_FLAGS(0),
            None, // today
            w!("ddd d MMM"),
            Some(&mut buffer),
            PCWSTR::null(),
        )
    };
    // `len` counts the terminating null; 0 means it failed.
    (len > 1).then(|| String::from_utf16_lossy(&buffer[..len as usize - 1]))
}

/// The bar whose window is `bar`.
fn find(bar: HWND) -> Option<Bar> {
    BARS.with_borrow(|bars| bars.iter().find(|b| b.bar == bar).copied())
}

/// Where the mouse is, from a mouse message's lParam: its low and high 16 bits, in
/// pixels from the window's top-left corner. Signed, as positions can be negative.
fn mouse_position(lparam: LPARAM) -> (i32, i32) {
    let x = i32::from(lparam.0 as u16 as i16);
    let y = i32::from((lparam.0 >> 16) as u16 as i16);
    (x, y)
}

/// `r` as Direct2D wants it.
fn rect_f(r: Rect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.x as f32,
        top: r.y as f32,
        right: (r.x + r.w) as f32,
        bottom: (r.y + r.h) as f32,
    }
}

unsafe extern "system" fn on_tip_message(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Nothing to handle: it's only ever shown, moved and hidden.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Where the cells go in a bar's window (inside the shadow), for a monitor scaled by
/// `scale`: the workspace bubbles, then the clock.
fn cells(scale: &impl Fn(i32) -> i32) -> Vec<Rect> {
    let shadow = scale(SHADOW);
    layout::bar_cells(
        &widths(scale),
        scale(BUBBLE_H),
        scale(settings::bar_padding()),
    )
    .into_iter()
    .map(|b| Rect {
        x: b.x + shadow,
        y: b.y + shadow,
        ..b
    })
    .collect()
}

/// Draws `b` with the current workspaces.
fn redraw(b: &Bar) {
    let at = b.rect;
    if let Err(err) = canvas::show(b.bar, at, |target| draw(target, b.monitor, at)) {
        log!("couldn't draw the workspace bar: {err}");
    }
}

/// The pill, then one bubble per workspace with its number, then a divider and the
/// clock, in pixels from the bar's top-left corner.
fn draw(target: &ID2D1RenderTarget, monitor: HMONITOR, at: Rect) -> Result<()> {
    let (current, occupied) = tracker::workspace_summary(monitor);
    let scale = scaler(monitor);
    let px = |size: i32| scale(size) as f32;
    let colors = settings::colors();
    let accent = COLORREF(colors.accent);
    unsafe {
        let brush = |c: COLORREF| target.CreateSolidColorBrush(&canvas::color(c, 1.0), None);
        let background = target.CreateSolidColorBrush(
            &canvas::color(COLORREF(colors.background), BACKGROUND_ALPHA),
            None,
        )?;
        let accent_brush = brush(accent)?;
        let empty = brush(COLORREF(colors.muted))?;
        let on_accent = brush(text_on(accent))?;
        let format = canvas::text_format(px(FONT), DWRITE_FONT_WEIGHT_SEMI_BOLD)?;

        // The bar: a pill as big as the window, less the shadow around it.
        let shadow = scale(SHADOW);
        let whole = Rect {
            x: shadow,
            y: shadow,
            w: at.w - 2 * shadow,
            h: at.h - 2 * shadow,
        };
        draw_shadow(target, whole, shadow)?;
        target.FillRoundedRectangle(&pill(whole, 0.0), &background);

        let line = px(LINE);
        let cells = cells(&scale);
        let bubbles = &cells[..tracker::WORKSPACES];
        let (volume, clock) = (cells[VOLUME], cells[CLOCK]);
        for (i, &b) in bubbles.iter().enumerate() {
            let number_brush = if occupied[i] {
                target.FillRoundedRectangle(&pill(b, 0.0), &accent_brush);
                &on_accent
            } else {
                // Lines are centered on the shape's edge; half a line in keeps it inside.
                outline(target, pill(b, -line / 2.0), &empty, line);
                &empty
            };
            if usize::from(current) == i + 1 {
                outline(
                    target,
                    pill(b, px(RING_GAP) + line / 2.0),
                    &accent_brush,
                    line,
                );
            }
            let number: Vec<u16> = (i + 1).to_string().encode_utf16().collect();
            target.DrawText(
                &number,
                &format,
                &rect_f(b),
                number_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }

        // A thin divider halfway between the last bubble and the volume.
        let last = bubbles[bubbles.len() - 1];
        let divider = ((last.x + last.w + volume.x) / 2) as f32;
        target.FillRectangle(
            &D2D_RECT_F {
                left: divider,
                top: (clock.y + scale(3)) as f32,
                right: divider + 1.0,
                bottom: (clock.y + clock.h - scale(3)) as f32,
            },
            &empty,
        );
        // The volume icon: muted (or no output device) in the quiet color.
        let level = volume::level();
        let icon: Vec<u16> = volume_icon(level).encode_utf16(&mut [0; 2]).to_vec();
        let icon_brush = match level {
            Some((_, false)) => brush(COLORREF(colors.text))?,
            _ => brush(COLORREF(colors.muted))?,
        };
        target.DrawText(
            &icon,
            &canvas::icon_format(px(ICON_SIZE))?,
            &rect_f(volume),
            &icon_brush,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
            DWRITE_MEASURING_MODE_NATURAL,
        );

        let now = GetLocalTime();
        let time: Vec<u16> = time_text(now.wHour, now.wMinute).encode_utf16().collect();
        target.DrawText(
            &time,
            &format,
            &rect_f(clock),
            &brush(COLORREF(colors.text))?,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }
    Ok(())
}

/// The volume tooltip's text: "45%", "Muted", or "No output device".
fn volume_text(level: Option<(f32, bool)>) -> String {
    match level {
        Some((level, false)) => format!("{}%", (level * 100.0).round()),
        Some((_, true)) => "Muted".to_string(),
        None => "No output device".to_string(),
    }
}

/// The icon (a character in Windows 11's icon font) for the volume: a speaker with
/// 0–3 sound waves by level, or crossed out when muted or there's no output device.
fn volume_icon(level: Option<(f32, bool)>) -> char {
    match level {
        Some((level, false)) if level >= 0.67 => '\u{E995}', // three waves
        Some((level, false)) if level >= 0.34 => '\u{E994}', // two
        Some((level, false)) if level > 0.0 => '\u{E993}',   // one
        Some((_, false)) => '\u{E992}',                      // none: volume at 0
        _ => '\u{E74F}',                                     // muted
    }
}

/// The clock's text: 12-hour "h:mm AM" (no leading zero on the hour).
fn time_text(hour: u16, minute: u16) -> String {
    let half = if hour < 12 { "AM" } else { "PM" };
    // 0 and 12 are both "12" on a 12-hour clock.
    let hour = match hour % 12 {
        0 => 12,
        h => h,
    };
    format!("{hour}:{minute:02} {half}")
}

/// A soft shadow `reach` pixels wide around the pill `r`: one-pixel rings, each fainter
/// than the one inside it. Rings rather than filled shapes, so the shadow doesn't
/// darken the (see-through) bar itself.
fn draw_shadow(target: &ID2D1RenderTarget, r: Rect, reach: i32) -> Result<()> {
    for i in 0..reach {
        // Fades to nothing, quickly at first: (1 - distance)².
        let fade = 1.0 - i as f32 / reach as f32;
        let alpha = SHADOW_ALPHA * fade * fade;
        let black =
            unsafe { target.CreateSolidColorBrush(&canvas::color(COLORREF(0), alpha), None)? };
        // Centered half a pixel out, so ring `i` covers exactly pixel row `i` outside.
        outline(target, pill(r, i as f32 + 0.5), &black, 1.0);
    }
    Ok(())
}

/// `r` as a pill (both ends fully round), grown by `by` pixels on every side (negative
/// shrinks it).
fn pill(r: Rect, by: f32) -> D2D1_ROUNDED_RECT {
    let radius = r.h as f32 / 2.0 + by;
    D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: r.x as f32 - by,
            top: r.y as f32 - by,
            right: (r.x + r.w) as f32 + by,
            bottom: (r.y + r.h) as f32 + by,
        },
        radiusX: radius,
        radiusY: radius,
    }
}

/// A pill's outline, `width` pixels thick.
fn outline(target: &ID2D1RenderTarget, shape: D2D1_ROUNDED_RECT, brush: &ID2D1Brush, width: f32) {
    unsafe { target.DrawRoundedRectangle(&shape, brush, width, None::<&ID2D1StrokeStyle>) };
}

/// Black or white, whichever reads better on `background`.
fn text_on(background: COLORREF) -> COLORREF {
    let c = background.0;
    let (r, g, b) = (c & 0xFF, (c >> 8) & 0xFF, (c >> 16) & 0xFF);
    // Perceived brightness: green looks brightest, blue darkest.
    if r * 299 + g * 587 + b * 114 > 128_000 {
        COLORREF(0x0000_0000)
    } else {
        COLORREF(0x00FF_FFFF)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_text_shows_percent_or_why_not() {
        assert_eq!(volume_text(Some((0.45, false))), "45%");
        assert_eq!(volume_text(Some((1.0, false))), "100%");
        assert_eq!(volume_text(Some((0.45, true))), "Muted");
        assert_eq!(volume_text(None), "No output device");
    }

    #[test]
    fn volume_icon_follows_level_and_mute() {
        assert_eq!(volume_icon(Some((1.0, false))), '\u{E995}');
        assert_eq!(volume_icon(Some((0.5, false))), '\u{E994}');
        assert_eq!(volume_icon(Some((0.1, false))), '\u{E993}');
        assert_eq!(volume_icon(Some((0.0, false))), '\u{E992}');
        assert_eq!(volume_icon(Some((0.8, true))), '\u{E74F}'); // muted
        assert_eq!(volume_icon(None), '\u{E74F}'); // no output device
    }

    #[test]
    fn time_is_12_hour_with_am_pm() {
        assert_eq!(time_text(9, 5), "9:05 AM");
        assert_eq!(time_text(0, 0), "12:00 AM"); // midnight
        assert_eq!(time_text(12, 30), "12:30 PM"); // noon
        assert_eq!(time_text(23, 59), "11:59 PM");
    }

    #[test]
    fn numbers_are_dark_on_light_bubbles_and_light_on_dark_ones() {
        assert_eq!(text_on(COLORREF(0x00FF_FFFF)), COLORREF(0)); // white
        assert_eq!(text_on(COLORREF(0x0000_FFFF)), COLORREF(0)); // yellow
        assert_eq!(text_on(COLORREF(0x0000_0000)), COLORREF(0x00FF_FFFF)); // black
        assert_eq!(text_on(COLORREF(0x00FF_0000)), COLORREF(0x00FF_FFFF)); // blue
    }
}
