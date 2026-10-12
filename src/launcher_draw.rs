//! Painting the launcher window with Direct2D (through `canvas::draw_on`): the search
//! box, then one row per matching app (icon + name), with the selected row highlighted.

use windows::Win32::Foundation::{COLORREF, POINT, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_PROPERTIES,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ROUNDED_RECT, ID2D1Brush, ID2D1RenderTarget,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_MEASURING_MODE_NATURAL, DWRITE_TEXT_ALIGNMENT_LEADING,
    IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HDC;
use windows::core::Result;

use crate::config::Favorite;
use crate::{canvas, icons, settings};

// Sizes in pixels at 100% scaling.
pub const WIDTH: i32 = 480;
pub const PADDING: i32 = 8;
const SEARCH_BOX_HEIGHT: i32 = 44;
/// The search box plus the gap below it.
pub const SEARCH_HEIGHT: i32 = SEARCH_BOX_HEIGHT + PADDING;
pub const ROW_HEIGHT: i32 = 48;
/// 32: a size apps ship icons in, as is 48 (32 at 150% scaling), so no stretching.
const ICON_SIZE: i32 = 32;
const SEARCH_FONT: i32 = 18;
const ROW_FONT: i32 = 15;

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
    let mut caret = POINT::default();
    let drawn = canvas::draw_on(dc, width, height, |target| {
        caret = draw_view(target, width, dpi, view)?;
        Ok(())
    });
    if let Err(err) = drawn {
        log!("couldn't draw the launcher: {err}");
    }
    caret
}

/// Draws everything. Returns where the text cursor goes.
fn draw_view(target: &ID2D1RenderTarget, width: i32, dpi: i32, view: &View) -> Result<POINT> {
    let s = |px: i32| px * dpi / 96;
    let colors = settings::colors();
    let brush =
        |c: u32| unsafe { target.CreateSolidColorBrush(&canvas::color(COLORREF(c), 1.0), None) };
    let accent = brush(colors.accent)?;
    let (text_brush, dim) = (brush(colors.text)?, brush(colors.muted)?);
    unsafe { target.Clear(Some(&canvas::color(COLORREF(colors.background), 1.0))) };

    // Search box, with an accent line along its bottom like Windows 11's text boxes:
    // the box's own rounded shape, drawn in the accent but only its bottom 2 pixels.
    let search = rect(
        s(PADDING),
        s(PADDING),
        width - s(PADDING),
        s(PADDING + SEARCH_BOX_HEIGHT),
    );
    let line = RECT {
        top: search.bottom - s(2),
        ..search
    };
    unsafe {
        target.FillRoundedRectangle(&rounded(search, s(6)), &brush(colors.surface)?);
        target.PushAxisAlignedClip(&rect_f(line), D2D1_ANTIALIAS_MODE_ALIASED);
        target.FillRoundedRectangle(&rounded(search, s(6)), &accent);
        target.PopAxisAlignedClip();
    }
    let search_text = RECT {
        left: search.left + s(14),
        right: search.right - s(14),
        ..search
    };
    let format = one_line_format(s(SEARCH_FONT))?;
    if view.query.is_empty() {
        text(target, search_text, "Search apps", &format, &dim);
    } else {
        text(target, search_text, view.query, &format, &text_brush);
    }
    let typed_width = canvas::text_width(view.query, &format)?.round() as i32;
    let (_, caret_height) = caret_size(dpi);
    let caret = POINT {
        x: (search_text.left + typed_width).min(search_text.right),
        y: (search.top + search.bottom - caret_height) / 2,
    };

    // App rows.
    let format = one_line_format(s(ROW_FONT))?;
    let selected_row = brush(colors.highlight)?;
    let icon_size = s(ICON_SIZE);
    let mut top = s(PADDING + SEARCH_HEIGHT);
    for (i, app) in view.rows.iter().enumerate() {
        let row = rect(s(PADDING), top, width - s(PADDING), top + s(ROW_HEIGHT));
        let middle = (row.top + row.bottom) / 2;
        if i == view.selected {
            unsafe { target.FillRoundedRectangle(&rounded(row, s(4)), &selected_row) };
            // Windows 11's small accent bar on the left of a selected item.
            let bar = rect(row.left, middle - s(8), row.left + s(3), middle + s(8));
            unsafe { target.FillRoundedRectangle(&rounded(bar, s(2)), &accent) };
        }
        let icon_x = row.left + s(12);
        if let Some(icon) = icons::for_target(&app.open, icon_size) {
            draw_icon(target, &icon, icon_x, middle - icon_size / 2, icon_size)?;
        }
        let label = RECT {
            left: icon_x + icon_size + s(12),
            right: row.right - s(12),
            ..row
        };
        text(target, label, &app.name, &format, &text_brush);
        top += s(ROW_HEIGHT);
    }
    if view.rows.is_empty() {
        let row = rect(
            s(PADDING + 12),
            top,
            width - s(PADDING),
            top + s(ROW_HEIGHT),
        );
        text(target, row, "No matching apps", &format, &dim);
    }
    Ok(caret)
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
    RECT {
        left,
        top,
        right,
        bottom,
    }
}

/// `r` as Direct2D wants it.
fn rect_f(r: RECT) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.left as f32,
        top: r.top as f32,
        right: r.right as f32,
        bottom: r.bottom as f32,
    }
}

/// `r` with corners rounded `radius` pixels.
fn rounded(r: RECT, radius: i32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect: rect_f(r),
        radiusX: radius as f32,
        radiusY: radius as f32,
    }
}

/// Windows 11's UI font, `px` pixels tall: one line, left-aligned, vertically centered,
/// cut off with "…" if too long.
fn one_line_format(px: i32) -> Result<IDWriteTextFormat> {
    let format = canvas::text_format(px as f32, DWRITE_FONT_WEIGHT_NORMAL)?;
    unsafe { format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)? };
    canvas::one_line(&format)?;
    Ok(format)
}

/// `s` in `r`, laid out by `format`.
fn text(
    target: &ID2D1RenderTarget,
    r: RECT,
    s: &str,
    format: &IDWriteTextFormat,
    brush: &ID2D1Brush,
) {
    let wide: Vec<u16> = s.encode_utf16().collect();
    unsafe {
        target.DrawText(
            &wide,
            format,
            &rect_f(r),
            brush,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }
}

/// Draws an icon (`size` × `size` premultiplied pixels from `icons::for_target`) with
/// its top-left corner at `x`, `y`, blending its see-through parts with what's under it.
fn draw_icon(target: &ID2D1RenderTarget, pixels: &[u8], x: i32, y: i32, size: i32) -> Result<()> {
    let properties = D2D1_BITMAP_PROPERTIES {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
    };
    let square = D2D_SIZE_U {
        width: size as u32,
        height: size as u32,
    };
    unsafe {
        // Copies the pixels into Direct2D's own bitmap; `size * 4` bytes per row.
        let bitmap = target.CreateBitmap(
            square,
            Some(pixels.as_ptr().cast()),
            (size * 4) as u32,
            &properties,
        )?;
        // Drawn at its own size, so no pixel is stretched.
        target.DrawBitmap(
            &bitmap,
            Some(&rect_f(rect(x, y, x + size, y + size))),
            1.0,
            D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
            None,
        );
    }
    Ok(())
}
