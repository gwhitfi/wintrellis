//! The workspace bar: a small pill at the top center of each monitor with one numbered
//! bubble per workspace. Filled = the workspace has windows, outlined = empty, ringed =
//! the one showing; clicking a bubble switches to its workspace. Always shown: the
//! band it sits in is kept free of windows (see `reserved_height`).

use std::cell::RefCell;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ROUNDED_RECT, ID2D1Brush, ID2D1RenderTarget, ID2D1StrokeStyle,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL,
};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, IDC_ARROW, LoadCursorW, RegisterClassW,
    SW_SHOWNOACTIVATE, ShowWindow, WM_LBUTTONDOWN, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{BOOL, Result, w};

use crate::layout::{self, Rect};
use crate::{canvas, settings, tiler, tracker};

// Sizes in pixels at 100% scaling.
/// Each workspace's pill.
const BUBBLE_W: i32 = 34;
const BUBBLE_H: i32 = 16;
// The space above the bar, inside it and below it are settings (`bar_margin`,
// `bar_padding`, `bar_gap`).
/// The current workspace's ring: its distance from the bubble, and its thickness.
const RING_GAP: i32 = 1;
const LINE: i32 = 2;
const FONT: i32 = 11;

/// One monitor's bar.
#[derive(Clone, Copy)]
struct Bar {
    monitor: HMONITOR,
    bar: HWND,
    /// Where the bar goes on screen.
    rect: Rect,
}

thread_local! {
    static BARS: RefCell<Vec<Bar>> = const { RefCell::new(Vec::new()) };
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
    }
    build()
}

/// Throws the bars away and builds them again: monitors were added, removed or
/// rescaled, the taskbar moved, or the `workspace_bar` setting changed.
pub fn rebuild() {
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
        let rect = layout::top_center(area, w, h, px(settings::bar_margin()));
        // Never focused, even when clicked. Layered, so `canvas::show` can give it
        // see-through, smooth edges; its fully see-through pixels let clicks through.
        let bar = new_window(rect)?;
        let b = Bar { monitor, bar, rect };
        BARS.with_borrow_mut(|bars| bars.push(b));
        // Drawn before it's shown, so it appears finished.
        redraw(&b);
        let _ = unsafe { ShowWindow(bar, SW_SHOWNOACTIVATE) };
    }
    Ok(())
}

/// Redraws every bar, after workspaces or windows changed.
pub fn refresh() {
    for b in BARS.with_borrow(|bars| bars.clone()) {
        redraw(&b);
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

/// The bar's (width, height) on a monitor scaled by `scale`.
fn size(scale: &impl Fn(i32) -> i32) -> (i32, i32) {
    layout::bar_size(
        tracker::WORKSPACES,
        scale(BUBBLE_W),
        scale(BUBBLE_H),
        scale(settings::bar_padding()),
    )
}

fn new_window(r: Rect) -> Result<HWND> {
    let instance = unsafe { GetModuleHandleW(None) }?;
    unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
            w!("WinTrellis-workspace-bar"),
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
    if msg == WM_LBUTTONDOWN {
        clicked(hwnd, lparam);
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// A click on `bar`: switches its monitor to the workspace whose bubble was hit.
fn clicked(bar: HWND, lparam: LPARAM) {
    let Some(b) = BARS.with_borrow(|bars| bars.iter().find(|b| b.bar == bar).copied()) else {
        return;
    };
    // lParam's low and high 16 bits: where the click was, from the bar's top-left
    // corner. Signed, as positions can be negative in general.
    let x = i32::from(lparam.0 as u16 as i16);
    let y = i32::from((lparam.0 >> 16) as u16 as i16);
    if let Some(i) = layout::rect_at(&bubbles(&scaler(b.monitor)), x, y) {
        tracker::switch_workspace_on(b.monitor, i as u8 + 1);
    }
}

/// Where the bubbles go in a bar, for a monitor scaled by `scale`.
fn bubbles(scale: &impl Fn(i32) -> i32) -> Vec<Rect> {
    layout::bar_bubbles(
        tracker::WORKSPACES,
        scale(BUBBLE_W),
        scale(BUBBLE_H),
        scale(settings::bar_padding()),
    )
}

/// Draws `b` with the current workspaces.
fn redraw(b: &Bar) {
    let at = b.rect;
    if let Err(err) = canvas::show(b.bar, at, |target| draw(target, b.monitor, at)) {
        log!("couldn't draw the workspace bar: {err}");
    }
}

/// The pill, then one bubble per workspace with its number, in pixels from the bar's
/// top-left corner.
fn draw(target: &ID2D1RenderTarget, monitor: HMONITOR, at: Rect) -> Result<()> {
    let (current, occupied) = tracker::workspace_summary(monitor);
    let scale = scaler(monitor);
    let px = |size: i32| scale(size) as f32;
    let colors = settings::colors();
    let accent = COLORREF(colors.accent);
    unsafe {
        let brush = |c: COLORREF| target.CreateSolidColorBrush(&canvas::color(c, 1.0), None);
        let background = brush(COLORREF(colors.background))?;
        let accent_brush = brush(accent)?;
        let empty = brush(COLORREF(colors.muted))?;
        let on_accent = brush(text_on(accent))?;
        let format = canvas::text_format(px(FONT), DWRITE_FONT_WEIGHT_SEMI_BOLD)?;

        // The bar: a pill as big as the window.
        let whole = Rect {
            x: 0,
            y: 0,
            w: at.w,
            h: at.h,
        };
        target.FillRoundedRectangle(&pill(whole, 0.0), &background);

        let line = px(LINE);
        for (i, b) in bubbles(&scale).into_iter().enumerate() {
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
                &D2D_RECT_F {
                    left: b.x as f32,
                    top: b.y as f32,
                    right: (b.x + b.w) as f32,
                    bottom: (b.y + b.h) as f32,
                },
                number_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
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
    fn numbers_are_dark_on_light_bubbles_and_light_on_dark_ones() {
        assert_eq!(text_on(COLORREF(0x00FF_FFFF)), COLORREF(0)); // white
        assert_eq!(text_on(COLORREF(0x0000_FFFF)), COLORREF(0)); // yellow
        assert_eq!(text_on(COLORREF(0x0000_0000)), COLORREF(0x00FF_FFFF)); // black
        assert_eq!(text_on(COLORREF(0x00FF_0000)), COLORREF(0x00FF_FFFF)); // blue
    }
}
