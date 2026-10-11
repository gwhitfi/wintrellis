//! Painting the launcher window with GDI: the search box, then one row per matching
//! app (icon + name), with the selected row highlighted.

use windows::Win32::Foundation::{COLORREF, POINT, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, AlphaBlend, BLENDFUNCTION, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
    CreateCompatibleDC, CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DEFAULT_CHARSET,
    DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW,
    FW_NORMAL, FillRect, GetStockObject, GetTextExtentPoint32W, HBITMAP, HDC, HFONT, NULL_PEN,
    OUT_DEFAULT_PRECIS, RoundRect, SelectClipRgn, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows::core::w;

use crate::config::Favorite;
use crate::{icons, settings};

// Sizes in pixels at 100% scaling.
pub const WIDTH: i32 = 480;
pub const PADDING: i32 = 8;
const SEARCH_BOX_HEIGHT: i32 = 44;
/// The search box plus the gap below it.
pub const SEARCH_HEIGHT: i32 = SEARCH_BOX_HEIGHT + PADDING;
pub const ROW_HEIGHT: i32 = 40;
const ICON_SIZE: i32 = 24;
const SEARCH_FONT: i32 = 18;
const ROW_FONT: i32 = 15;

// Colors, as 0x00BBGGRR. Close to Windows 11's dark theme.
pub const BACKGROUND: COLORREF = COLORREF(0x0020_2020);
const SEARCH_BOX: COLORREF = COLORREF(0x002C_2C2C);
const SELECTED_ROW: COLORREF = COLORREF(0x0035_3535);
const TEXT: COLORREF = COLORREF(0x00FF_FFFF);
const DIM_TEXT: COLORREF = COLORREF(0x009A_9A9A);

/// What's on screen.
pub struct View<'a> {
    pub query: &'a str,
    /// The matching apps, best first.
    pub rows: Vec<&'a Favorite>,
    /// Index into `rows` of the highlighted one.
    pub selected: usize,
}

/// The blinking text cursor's size in pixels: (width, height).
pub fn caret_size(dpi: i32) -> (i32, i32) {
    ((dpi / 96).max(1), (SEARCH_FONT + 6) * dpi / 96)
}

/// Paints the whole window (`width` x `height` pixels) into `dc`. Returns where the
/// text cursor goes: just after the typed text.
pub fn draw(dc: HDC, width: i32, height: i32, dpi: i32, view: &View) -> POINT {
    let s = |px: i32| px * dpi / 96;
    let accent = COLORREF(settings::border_color());
    fill(dc, rect(0, 0, width, height), 0, BACKGROUND);

    // Search box, with an accent line along its bottom like Windows 11's text boxes.
    let search = rect(
        s(PADDING),
        s(PADDING),
        width - s(PADDING),
        s(PADDING + SEARCH_BOX_HEIGHT),
    );
    fill(dc, search, s(6), SEARCH_BOX);
    clipped_to_round_rect(dc, search, s(6), || {
        let line = RECT {
            top: search.bottom - s(2),
            ..search
        };
        fill(dc, line, 0, accent);
    });
    let search_text = RECT {
        left: search.left + s(14),
        right: search.right - s(14),
        ..search
    };
    let font = make_font(s(SEARCH_FONT));
    if view.query.is_empty() {
        text(dc, search_text, "Search apps", font, DIM_TEXT);
    } else {
        text(dc, search_text, view.query, font, TEXT);
    }
    let typed_width = text_width(dc, view.query, font);
    let (_, caret_height) = caret_size(dpi);
    let caret = POINT {
        x: (search_text.left + typed_width).min(search_text.right),
        y: (search.top + search.bottom - caret_height) / 2,
    };
    delete_font(font);

    // App rows.
    let font = make_font(s(ROW_FONT));
    let icon_size = s(ICON_SIZE);
    let mut top = s(PADDING + SEARCH_HEIGHT);
    for (i, app) in view.rows.iter().enumerate() {
        let row = rect(s(PADDING), top, width - s(PADDING), top + s(ROW_HEIGHT));
        let middle = (row.top + row.bottom) / 2;
        if i == view.selected {
            fill(dc, row, s(4), SELECTED_ROW);
            // Windows 11's small accent bar on the left of a selected item.
            let bar = rect(row.left, middle - s(8), row.left + s(3), middle + s(8));
            fill(dc, bar, s(2), accent);
        }
        let icon_x = row.left + s(12);
        if let Some(icon) = icons::for_target(&app.open, icon_size) {
            draw_icon(dc, icon, icon_x, middle - icon_size / 2, icon_size);
        }
        let label = RECT {
            left: icon_x + icon_size + s(12),
            right: row.right - s(12),
            ..row
        };
        text(dc, label, &app.name, font, TEXT);
        top += s(ROW_HEIGHT);
    }
    if view.rows.is_empty() {
        let row = rect(
            s(PADDING + 12),
            top,
            width - s(PADDING),
            top + s(ROW_HEIGHT),
        );
        text(dc, row, "No matching apps", font, DIM_TEXT);
    }
    delete_font(font);
    caret
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
    RECT {
        left,
        top,
        right,
        bottom,
    }
}

/// Fills `r` with `color`, with rounded corners if `radius` > 0.
fn fill(dc: HDC, r: RECT, radius: i32, color: COLORREF) {
    unsafe {
        let brush = CreateSolidBrush(color);
        if radius == 0 {
            FillRect(dc, &r, brush);
        } else {
            let old_brush = SelectObject(dc, brush.into());
            let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
            // With no outline, RoundRect leaves out the right and bottom edge, hence +1.
            let _ = RoundRect(
                dc,
                r.left,
                r.top,
                r.right + 1,
                r.bottom + 1,
                2 * radius,
                2 * radius,
            );
            SelectObject(dc, old_pen);
            SelectObject(dc, old_brush);
        }
        let _ = DeleteObject(brush.into());
    }
}

/// Runs `paint` with drawing limited to the rounded rectangle `r`.
fn clipped_to_round_rect(dc: HDC, r: RECT, radius: i32, paint: impl FnOnce()) {
    unsafe {
        let region = CreateRoundRectRgn(
            r.left,
            r.top,
            r.right + 1,
            r.bottom + 1,
            2 * radius,
            2 * radius,
        );
        SelectClipRgn(dc, Some(region));
        paint();
        SelectClipRgn(dc, None);
        let _ = DeleteObject(region.into());
    }
}

/// Windows 11's UI font, `px` pixels tall.
fn make_font(px: i32) -> HFONT {
    unsafe {
        // A negative height means the size of the letters themselves, not the line.
        CreateFontW(
            -px,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI Variable Text"),
        )
    }
}

fn delete_font(font: HFONT) {
    let _ = unsafe { DeleteObject(font.into()) };
}

/// One line of text, vertically centered in `r`, cut off with "..." if too long.
fn text(dc: HDC, mut r: RECT, s: &str, font: HFONT, color: COLORREF) {
    let mut wide: Vec<u16> = s.encode_utf16().collect();
    unsafe {
        let old = SelectObject(dc, font.into());
        SetTextColor(dc, color);
        SetBkMode(dc, TRANSPARENT);
        // NOPREFIX: show "&" as is (by default it would underline the next letter).
        DrawTextW(
            dc,
            &mut wide,
            &mut r,
            DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
        SelectObject(dc, old);
    }
}

/// How wide `s` is in pixels, in `font`.
fn text_width(dc: HDC, s: &str, font: HFONT) -> i32 {
    let wide: Vec<u16> = s.encode_utf16().collect();
    let mut size = SIZE::default();
    unsafe {
        let old = SelectObject(dc, font.into());
        let _ = GetTextExtentPoint32W(dc, &wide, &mut size);
        SelectObject(dc, old);
    }
    size.cx
}

/// Draws an icon bitmap, blending its see-through parts with what's underneath.
fn draw_icon(dc: HDC, icon: HBITMAP, x: i32, y: i32, size: i32) {
    unsafe {
        // GDI copies between DCs, so the icon needs a DC of its own to copy from.
        let source = CreateCompatibleDC(Some(dc));
        let old = SelectObject(source, icon.into());
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            // Use each pixel's own transparency.
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let _ = AlphaBlend(dc, x, y, size, size, source, 0, 0, size, size, blend);
        SelectObject(source, old);
        let _ = DeleteDC(source);
    }
}
